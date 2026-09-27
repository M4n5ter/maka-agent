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

use super::Editor;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Command,
    Reference,
}

/// An exact editing position, not permission to execute a selected candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub range: Range<usize>,
    pub query: String,
    pub arguments: String,
    revision: u64,
    cursor: usize,
    anchor: Option<usize>,
    manual: bool,
}

impl Token {
    pub(super) fn matches(&self, editor: &Editor) -> bool {
        if self.manual {
            self.revision == editor.revision
                && self.cursor == editor.selection.cursor
                && self.anchor == editor.selection.anchor
                && self.range == (self.cursor..self.cursor)
        } else {
            editor.completion().as_ref() == Some(self)
        }
    }
}

impl Editor {
    /// Opening a source picker does not edit the draft or its undo history.
    pub fn insertion_token(&self, kind: Kind) -> Token {
        Token {
            kind,
            range: self.selection.cursor..self.selection.cursor,
            query: String::new(),
            arguments: String::new(),
            revision: self.revision,
            cursor: self.selection.cursor,
            anchor: self.selection.anchor,
            manual: true,
        }
    }
    pub fn completion(&self) -> Option<Token> {
        if !self.selection.range().is_empty() {
            return None;
        }
        let cursor = self.selection.cursor;
        let before = &self.text[..cursor];
        let start = before.len() - before.trim_start().len();
        if let Some(command) = before[start..].strip_prefix('/') {
            // A path remains ordinary text. Command arguments are owned by the
            // selected command's normal form, never a shell parser.
            let end = command.find(char::is_whitespace).unwrap_or(command.len());
            let query = &command[..end];
            if query
                .chars()
                .all(|ch| ch.is_alphanumeric() || "-_.:".contains(ch))
            {
                return Some(Token {
                    kind: Kind::Command,
                    range: start..start + 1 + end,
                    query: query.to_owned(),
                    arguments: command[end..].trim_start().to_owned(),
                    revision: self.revision,
                    cursor,
                    anchor: self.selection.anchor,
                    manual: false,
                });
            }
        }
        let start = before.rfind('@')?;
        if before[..start]
            .chars()
            .next_back()
            .is_some_and(|ch| !ch.is_whitespace() && !"([{>".contains(ch))
        {
            return None;
        }
        let query = &before[start + 1..];
        if query.chars().any(char::is_whitespace) || in_code(&before[..start]) {
            return None;
        }
        Some(Token {
            kind: Kind::Reference,
            range: start..cursor,
            query: query.to_owned(),
            arguments: String::new(),
            revision: self.revision,
            cursor,
            anchor: self.selection.anchor,
            manual: false,
        })
    }

    /// A candidate may only replace the same token the user actually reviewed.
    /// Its insertion is one ordinary undoable edit; dispatch remains with App.
    pub fn complete(&mut self, token: &Token, inserted: &str) -> bool {
        if !token.matches(self)
            || inserted.chars().any(|ch| ch.is_control())
            || self.text.len() - token.range.len() + inserted.len() > self.byte_limit
        {
            return false;
        }
        self.replace(token.range.clone(), inserted);
        true
    }
}

fn in_code(text: &str) -> bool {
    let mut fence = None;
    let mut inline = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if inline.is_none() && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            let marker = trimmed.as_bytes()[0];
            let count = trimmed.bytes().take_while(|byte| *byte == marker).count();
            match fence {
                Some((open, length)) if open == marker && count >= length => fence = None,
                None => fence = Some((marker, count)),
                _ => {}
            }
        } else if fence.is_none() {
            let bytes = line.as_bytes();
            let mut at = 0;
            while at < bytes.len() {
                if bytes[at] == b'\\' && inline.is_none() {
                    at += 2;
                } else if bytes[at] == b'`' {
                    let count = bytes[at..].iter().take_while(|byte| **byte == b'`').count();
                    inline = match inline {
                        Some(length) if length == count => None,
                        Some(length) => Some(length),
                        None => Some(count),
                    };
                    at += count;
                } else {
                    at += 1;
                }
            }
        }
    }
    fence.is_some() || inline.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn completion_distinguishes_commands_references_and_literal_text() {
        for text in [
            "email@example.org",
            "https://site/@name",
            "`@code",
            "`first\n@code",
            "``code ` @text",
            "```rust\n@code",
            "/tmp/file",
        ] {
            let mut editor = Editor::default();
            editor.insert(text);
            assert!(editor.completion().is_none(), "{text}");
        }
        for (text, kind, query, args) in [
            ("/model gpt-6", Kind::Command, "model", "gpt-6"),
            ("  /技能", Kind::Command, "技能", ""),
            ("引用 @src/main.rs", Kind::Reference, "src/main.rs", ""),
            ("```rust\ncode\n```\n@文档", Kind::Reference, "文档", ""),
        ] {
            let mut editor = Editor::default();
            editor.insert(text);
            let token = editor.completion().unwrap();
            assert_eq!(
                (token.kind, token.query.as_str(), token.arguments.as_str()),
                (kind, query, args)
            );
        }
    }

    #[test]
    fn an_explicit_picker_has_no_draft_effect_until_a_current_selection_is_chosen() {
        let mut editor = Editor::default();
        editor.insert("原消息");
        let token = editor.insertion_token(Kind::Reference);
        let revision = editor.revision;
        assert_eq!(editor.text(), "原消息");
        assert_eq!(editor.revision, revision);
        editor.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(!editor.complete_marked(&token, "@stale", "stale"));
        let token = editor.insertion_token(Kind::Reference);
        assert!(editor.complete_marked(&token, "@文档", "chosen"));
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), "原消息");
        assert!(editor.marks().is_empty());
    }

    #[test]
    fn a_late_candidate_cannot_replace_reedited_or_moved_text_and_unicode_undo_is_exact() {
        let mut editor = Editor::default();
        editor.insert("你好 🦀 @文");
        let original = editor.text().to_owned();
        let old = editor.completion().unwrap();
        editor.insert("件");
        assert!(!editor.complete(&old, "@wrong"));
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), original);
        assert!(
            !editor.complete(&old, "@old"),
            "undo does not revive an old async request"
        );
        let current = editor.completion().unwrap();
        assert!(editor.complete(&current, "@文档🦀"));
        assert_eq!(editor.text(), "你好 🦀 @文档🦀");
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), original);
        let current = editor.completion().unwrap();
        editor.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(!editor.complete(&current, "@wrong-position"));
    }
}
