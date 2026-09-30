/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

//! Explicit clipboard reads and terminal-delivered file paths share attachment admission.
use super::{App, ConnectionState, LIMIT, Saved};
use crate::{
    app::{Focus, Notice},
    editor::saved::Cursor,
    navigation::Route,
};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    id: String,
    pub session: String,
    root: String,
    epoch: String,
    revision: u64,
    cursor: Cursor,
}
pub(crate) struct Pending {
    pub ticket: Ticket,
    requested: bool,
}
pub(crate) enum Content {
    Text(String),
    Files(Vec<PathBuf>),
    Image(tempfile::NamedTempFile),
}

/// Interpret only a whole paste made of local paths; never run shell expansion.
pub(crate) fn paths(text: &str) -> Option<Vec<PathBuf>> {
    let text = text.trim();
    if text.is_empty() || text.len() > 8 * 4096 {
        return None;
    }
    let words = path_words(text, cfg!(windows))?;
    let parsed: Vec<_> = words
        .iter()
        .map(|word| local_path(word))
        .collect::<Option<_>>()?;
    (!parsed.is_empty()).then_some(parsed)
}

/// Path quoting, not a shell language: comments, operators and expansions stay
/// literal tokens, so a prose suffix can never disappear during classification.
fn path_words(text: &str, windows: bool) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if quote.is_none() && word.starts_with("file://") && !character.is_whitespace() {
            // URI apostrophes are path characters, not shell quote delimiters.
            word.push(character);
        } else if quote == Some('\'') {
            if character == '\'' {
                quote = None;
            } else {
                word.push(character);
            }
        } else if character == '\\' && !windows {
            let escaped = chars.next()?;
            if matches!(escaped, '\n' | '\r') {
                return None;
            }
            if quote == Some('"') && !matches!(escaped, '\\' | '"' | '$' | '`') {
                word.push('\\');
            }
            word.push(escaped);
            started = true;
        } else if Some(character) == quote {
            quote = None;
        } else if quote.is_none() && (character == '"' || character == '\'' && !windows) {
            quote = Some(character);
            started = true;
        } else if quote.is_none() && character.is_whitespace() {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(character);
            started = true;
        }
    }
    if started {
        words.push(word);
    }
    quote.is_none().then_some(words)
}
fn local_path(text: &str) -> Option<PathBuf> {
    let path = if text.starts_with("file://") {
        if text.chars().any(char::is_whitespace) {
            return None;
        }
        let url = url::Url::parse(text).ok()?;
        if url.host_str().is_some_and(|host| host != "localhost")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        url.to_file_path().ok()?
    } else {
        PathBuf::from(text)
    };
    super::io::valid_path(&path).then_some(path)
}

impl App {
    /// No filesystem work on the input thread; existing attachment preparation
    /// reports unreadable paths, directories and oversized files on their chips.
    pub(crate) fn paste_text(&mut self, text: &str) -> bool {
        let Route::Session(session) = self.navigation.current() else {
            return false;
        };
        self.paste_into(&session, text)
    }
    fn paste_into(&mut self, session: &str, text: &str) -> bool {
        if let Some(paths) = paths(text)
            && self.attachment_editable(session)
        {
            match self.attach_paths(session, paths) {
                Ok(()) => return true,
                Err(key) => self.notice = Some(Notice::Local(key)),
            }
        }
        self.drafts
            .get_mut(session)
            .is_some_and(|editor| editor.insert(text))
    }
    fn attach_paths(&mut self, session: &str, paths: Vec<PathBuf>) -> Result<(), &'static str> {
        if paths.is_empty()
            || !self.attachment_editable(session)
            || paths.iter().any(|path| !super::io::valid_path(path))
        {
            return Err("attachments-invalid");
        }
        let items = self.attachments.saved.entry(session.into()).or_default();
        let mut unique = Vec::new();
        for path in paths {
            if !items.iter().any(|item| item.path == path) && !unique.contains(&path) {
                unique.push(path);
            }
        }
        if items.len() + unique.len() > LIMIT {
            return Err("attachments-limit");
        }
        for path in unique {
            let id = uuid::Uuid::new_v4().to_string();
            items.push(Saved {
                id: id.clone(),
                path,
                manifest: None,
                attachment: None,
            });
            self.attachments
                .queued
                .push_back((session.into(), None, id));
        }
        Ok(())
    }
    pub(crate) fn paste_clipboard(&mut self) {
        if self.attachments.paste.is_some() {
            return;
        }
        let Route::Session(session) = self.navigation.current() else {
            return;
        };
        if !self.attachment_editable(&session) {
            return;
        }
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return;
        };
        let editor = &self.drafts[&session];
        self.attachments.paste = Some(Pending {
            ticket: Ticket {
                id: uuid::Uuid::new_v4().to_string(),
                session,
                root: root_id.clone(),
                epoch: epoch.clone(),
                revision: editor.revision(),
                cursor: editor.cursor(),
            },
            requested: false,
        });
        if matches!(self.notice, Some(Notice::Paste(_))) {
            self.notice = None;
        }
        self.focus = Focus::Composer;
    }
    pub(crate) fn clipboard_request(&mut self) -> Option<Ticket> {
        let pending = self.attachments.paste.as_mut()?;
        if pending.requested || self.closing {
            return None;
        }
        pending.requested = true;
        Some(pending.ticket.clone())
    }
    pub(crate) fn clipboard_pasted(&mut self, ticket: Ticket, result: Result<Content, String>) {
        if self
            .attachments
            .paste
            .as_ref()
            .is_none_or(|p| p.ticket != ticket)
        {
            return;
        }
        self.attachments.paste = None;
        if self.closing
            || !self.attachment_editable(&ticket.session)
            || !matches!(&self.connection, ConnectionState::Connected {root_id, epoch}
                if *root_id == ticket.root && *epoch == ticket.epoch)
        {
            return;
        }
        let result = match result {
            Ok(Content::Files(paths)) => self
                .attach_paths(&ticket.session, paths)
                .map_err(|key| self.i18n.text(key)),
            Ok(Content::Image(file)) => {
                if self.attachment_files(&ticket.session, None).len() >= LIMIT {
                    Err(self.i18n.text("attachments-limit"))
                } else {
                    file.keep()
                        .map_err(|error| error.error.to_string())
                        .and_then(|(_, path)| {
                            self.attach_paths(&ticket.session, vec![path])
                                .map_err(|key| self.i18n.text(key))
                        })
                }
            }
            Ok(Content::Text(text)) => {
                let editor = self.drafts.get_mut(&ticket.session).unwrap();
                if editor.revision() != ticket.revision {
                    Err(self.i18n.text("composer-paste-changed"))
                } else {
                    // Honor the original insertion point, even after tab navigation.
                    let _ = editor.restore_cursor(ticket.cursor);
                    self.paste_into(&ticket.session, &text);
                    Ok(())
                }
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.notice = Some(Notice::Paste(error));
        }
    }
}

pub(crate) async fn read(ticket: Ticket, directory: Option<PathBuf>) -> super::Completed {
    let result = tokio::task::spawn_blocking(move || read_native(directory.as_deref()))
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
    super::Completed::Pasted(ticket, result)
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn read_native(directory: Option<&Path>) -> Result<Content, String> {
    use clipboard_rs::{Clipboard, ClipboardContext, ContentFormat, common::RustImage};
    let clipboard = ClipboardContext::new().map_err(|error| error.to_string())?;
    if clipboard.has(ContentFormat::Files) {
        let files = clipboard.get_files().map_err(|error| error.to_string())?;
        if files.len() > LIMIT {
            return Err(format!("Clipboard contains more than {LIMIT} files"));
        }
        let paths: Option<Vec<_>> = files.iter().map(|file| local_path(file)).collect();
        return paths
            .filter(|paths| !paths.is_empty())
            .map(Content::Files)
            .ok_or_else(|| "Clipboard contains no supported local file paths".into());
    }
    if clipboard.has(ContentFormat::Image) {
        let image = clipboard.get_image().map_err(|error| error.to_string())?;
        let (width, height) = image.get_size();
        if u64::from(width) * u64::from(height) > 32 * 1024 * 1024 {
            return Err("Clipboard image exceeds 32 megapixels".into());
        }
        let png = image.to_png().map_err(|error| error.to_string())?;
        return store_image(png.get_bytes(), directory).map(Content::Image);
    }
    if clipboard.has(ContentFormat::Text) {
        return clipboard
            .get_text()
            .map(Content::Text)
            .map_err(|error| error.to_string());
    }
    Err("Clipboard contains no image, local files or plain text".into())
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn read_native(_: Option<&Path>) -> Result<Content, String> {
    Err(
        "Native clipboard is unavailable on this platform; use terminal paste or Attach files"
            .into(),
    )
}
fn store_image(png: &[u8], directory: Option<&Path>) -> Result<tempfile::NamedTempFile, String> {
    if png.len() as u64 > maka_protocol::artifact::MAX_ATTACHMENT_BYTES {
        return Err("Clipboard image exceeds the attachment size limit".into());
    }
    let directory = directory.ok_or("Local draft storage is unavailable")?;
    let mut file = tempfile::Builder::new()
        .prefix("clipboard-")
        .suffix(".png")
        .tempfile_in(directory)
        .map_err(|error| error.to_string())?;
    file.write_all(png)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::Action,
        i18n::{I18n, Locale, LocalePreference},
    };
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn app() -> App {
        let mut app = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Auto, Locale::En),
        );
        app.connection = ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "epoch".into(),
        };
        app.apply(Action::Visit(Route::Session("a".into())));
        app.focus = Focus::Composer;
        app
    }
    fn paste_key() -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL))
    }
    #[test]
    fn path_pastes_preserve_unicode_spaces_and_never_evaluate_shell_syntax() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("中文 image.png");
        let second = directory.path().join("it's a file.txt");
        let quoted = format!("\"{}\" \"{}\"", first.display(), second.display());
        #[cfg(not(windows))]
        assert_eq!(paths(&quoted), Some(vec![first.clone(), second.clone()]));
        let uri_list = format!(
            "{}\r\n{}",
            url::Url::from_file_path(&first).unwrap(),
            url::Url::from_file_path(&second).unwrap()
        );
        assert_eq!(paths(&uri_list), Some(vec![first.clone(), second.clone()]));
        assert_eq!(
            paths(&uri_list.replace("\r\n", " ")),
            Some(vec![first.clone(), second.clone()])
        );
        assert!(
            paths(first.to_str().unwrap()).is_none(),
            "unquoted prose is not guessed to be a path containing spaces"
        );
        for text in [
            "Explain /tmp/file.txt",
            "hello\nworld",
            "file://remote.example/file.txt",
            "file:///tmp/a%00b",
            "$(touch /tmp/not-an-operation)",
            "relative.txt",
            "/tmp/a.txt # explain this file",
            "/tmp/a.txt\n# notes",
            "file:///tmp/a.txt please explain",
            "\"file:///tmp/a b\"",
            "/tmp/a.txt \"\"",
        ] {
            assert!(paths(text).is_none(), "{text}");
        }
        #[cfg(not(windows))]
        for text in ["'/tmp/file # name.txt'", r"/tmp/file\ \#\ name.txt"] {
            assert_eq!(
                paths(text),
                Some(vec![PathBuf::from("/tmp/file # name.txt")])
            );
        }
        assert_eq!(
            path_words(r#""C:\My Files\a.png" "D:\中文\b.txt""#, true),
            Some(vec![r"C:\My Files\a.png".into(), r"D:\中文\b.txt".into()])
        );
    }
    #[test]
    fn terminal_drops_use_the_attachment_queue_and_capacity_failure_preserves_text() {
        let mut app = app();
        for text in [
            "/tmp/a.txt # explain this file",
            "/tmp/a.txt\n# notes",
            "file:///tmp/a.txt please explain",
        ] {
            app.input(Event::Paste(text.into()));
            assert_eq!(app.drafts["a"].text(), text);
            assert!(app.attachment_files("a", None).is_empty());
            app.drafts.get_mut("a").unwrap().clear();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        app.drafts.get_mut("a").unwrap().insert("Describe this: ");
        app.input(Event::Paste(path.to_str().unwrap().into()));
        assert_eq!(app.drafts["a"].text(), "Describe this: ");
        assert_eq!(app.attachment_files("a", None).len(), 1);
        assert!(!app.attachments.ready("a"));
        assert!(!app.enabled(&Action::SendMessage));
        let (_, saved, _) = app.attachment_read_request().unwrap();
        assert_eq!(saved.path, path);
        app.input(Event::Paste(path.to_str().unwrap().into()));
        assert_eq!(
            app.attachment_files("a", None).len(),
            1,
            "duplicate paths do not create duplicate uploads"
        );
        let rest: Vec<_> = (1..LIMIT)
            .map(|i| directory.path().join(format!("{i}.txt")))
            .collect();
        app.attach_paths("a", rest).unwrap();
        let overflow = directory
            .path()
            .join("overflow.txt")
            .to_string_lossy()
            .into_owned();
        app.input(Event::Paste(overflow.clone()));
        assert_eq!(app.attachment_files("a", None).len(), LIMIT);
        assert!(app.drafts["a"].text().ends_with(&overflow));
        assert!(matches!(
            app.notice,
            Some(Notice::Local("attachments-limit"))
        ));
    }
    #[test]
    fn clipboard_results_belong_to_the_original_session_and_stale_images_are_removed() {
        let mut app = app();
        app.input(paste_key());
        let ticket = app.clipboard_request().unwrap();
        assert!(!app.attachments.ready("a"));
        assert!(
            app.attachments.has("a"),
            "an empty draft with a pending paste cannot be evicted"
        );
        assert!(app.clipboard_request().is_none());
        app.apply(Action::Visit(Route::Session("b".into())));
        let directory = tempfile::tempdir().unwrap();
        let file = store_image(b"\x89PNG\r\n\x1a\n", Some(directory.path())).unwrap();
        let path = file.path().to_owned();
        app.clipboard_pasted(ticket, Ok(Content::Image(file)));
        assert_eq!(app.attachment_files("a", None)[0].path, path);
        assert!(path.exists());
        assert!(app.attachment_files("b", None).is_empty());
        app.focus = Focus::Composer;
        app.input(paste_key());
        let old = app.clipboard_request().unwrap();
        app.attachments.disconnect();
        let file = store_image(b"png", Some(directory.path())).unwrap();
        let rejected = file.path().to_owned();
        app.clipboard_pasted(old, Ok(Content::Image(file)));
        assert!(!rejected.exists());
        assert!(app.attachment_files("b", None).is_empty());
    }
    #[test]
    fn clipboard_text_keeps_its_insertion_point_and_never_overwrites_later_edits() {
        let mut app = app();
        app.input(Event::Paste("before after".into()));
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::CONTROL,
        )));
        app.input(paste_key());
        let ticket = app.clipboard_request().unwrap();
        app.input(Event::Key(KeyEvent::new(
            KeyCode::End,
            KeyModifiers::CONTROL,
        )));
        app.clipboard_pasted(ticket, Ok(Content::Text("中文\n".into())));
        assert_eq!(app.drafts["a"].text(), "中文\nbefore after");
        app.input(paste_key());
        let ticket = app.clipboard_request().unwrap();
        app.input(Event::Paste("new edit".into()));
        let draft = app.drafts["a"].text().to_owned();
        app.clipboard_pasted(ticket, Ok(Content::Text("stale".into())));
        assert_eq!(app.drafts["a"].text(), draft);
        assert!(matches!(app.notice, Some(Notice::Paste(_))));
        app.apply(Action::Help);
        app.input(paste_key());
        assert!(
            app.clipboard_request().is_none(),
            "modal input does not read the clipboard"
        );
    }
}
