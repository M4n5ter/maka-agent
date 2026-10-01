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

//! Native terminal client. Host business logic remains outside this crate.
mod app;
mod apps;
mod chrome;
mod editor;
mod files;
mod i18n;
mod motion;
mod navigation;
mod overlay;
mod pages;
mod providers;
mod shutdown;
mod state;
mod terminal;
mod theme;
mod ui;
mod view;
mod welcome;

use app::{Action, App, ConnectionState, Notice};
use crossterm::event::EventStream;
use futures_util::StreamExt;
pub use i18n::{Locale, LocalePreference};
use maka_client::{Client, Error, Notification};
use maka_protocol::Operation;
use serde_json::{Value, json};
pub use shutdown::{ShutdownOutcome, ShutdownRequest};
use std::{path::PathBuf, time::Duration};
use tokio::{sync::mpsc, task::JoinSet};

pub struct Options {
    pub root: PathBuf,
    pub locale: Option<LocalePreference>,
    pub profile: String,
}

enum Completed {
    ComposerCut(
        pages::actions::native::EditorTarget,
        Result<(), &'static str>,
    ),
    Plugins(
        pages::plugins::Request,
        Result<pages::plugins::Output, maka_client::RequestFailure>,
    ),
    Providers(u64, Result<maka_client::ProviderDirectory, String>),
    SandboxDefaults(
        pages::manage::sandbox::defaults::Request,
        Result<maka_protocol::configuration::policy::RuntimePolicySnapshot, String>,
    ),
    Extension(Box<apps::Request>, Result<apps::Output, apps::io::Failure>),
    Preferences(
        pages::manage::preferences::Ticket,
        Result<pages::manage::preferences::Response, maka_client::RequestFailure>,
    ),
    Resources(
        pages::resources::Request,
        Result<pages::resources::Output, pages::resources::Failure>,
    ),
    NativeServices(
        maka_client::HostIdentity,
        Result<maka_client::NativeServices, maka_client::RequestFailure>,
    ),
    SessionControls(
        pages::session_controls::Request,
        Result<pages::session_controls::Output, maka_client::RequestFailure>,
    ),
    SessionAnchor(
        pages::session_controls::anchor::Request,
        Result<maka_client::transcript::TranscriptBatch, String>,
    ),
    Bundle(
        pages::bundle::Request,
        Result<pages::bundle::Output, maka_client::RequestFailure>,
    ),
    Resumed(
        pages::resume::Request,
        Result<pages::resume::Output, maka_client::RequestFailure>,
    ),
    Revised(
        pages::revision::Request,
        Result<pages::revision::Output, maka_client::RequestFailure>,
    ),
    Branched(
        pages::branch::Request,
        Result<pages::branch::Output, maka_client::RequestFailure>,
    ),
    Removal(
        pages::manage::removal::Request,
        Result<pages::manage::removal::Output, maka_client::RequestFailure>,
    ),
    Oauth(
        Box<pages::manage::oauth::Request>,
        Result<pages::manage::oauth::Output, maka_client::RequestFailure>,
    ),
    Managed(
        Box<pages::manage::Ticket>,
        Result<pages::manage::Updated, maka_client::RequestFailure>,
    ),
    History(
        pages::chat::history::Request,
        Result<pages::chat::history::Output, String>,
    ),
    Queue(
        pages::queue::Ticket,
        Result<maka_protocol::message::MutationResult, maka_client::RequestFailure>,
    ),
    Stopped(
        pages::chat::stopping::Target,
        Result<maka_protocol::turn::TurnSnapshot, maka_client::RequestFailure>,
    ),
    Context(
        pages::chat::context::Request,
        Result<maka_protocol::context::ContextDiagnosticsResult, String>,
    ),
    Interaction(
        Box<(
            pages::interactions::Ticket,
            Result<maka_protocol::interaction::InteractionSnapshot, maka_client::RequestFailure>,
        )>,
    ),
    Reconciled(
        pages::sending::Submission,
        Result<Option<maka_protocol::message::ExecutionResolution>, maka_client::RequestFailure>,
        Option<bool>,
    ),
    Submitted(
        pages::sending::Submission,
        Result<maka_protocol::message::SubmitResult, maka_client::RequestFailure>,
        bool,
    ),
    ChatOpened(
        pages::chat::OpenRequest,
        Result<Box<pages::chat::Opened>, pages::chat::OpenError>,
    ),
    ChatPage(
        pages::chat::PageRequest,
        Result<maka_client::transcript::TranscriptBatch, String>,
    ),
    ChatReady(String, Result<(), String>),
    ObservationClosed,
    Connected(Result<(Client, mpsc::Receiver<Notification>), Error>),
    Status(Result<Value, String>),
    Catalog(Result<maka_protocol::session::SessionCatalogQueryResult, String>),
    PaletteSessions(
        pages::commands::SessionSearchRequest,
        Result<maka_protocol::session::SessionCatalogQueryResult, String>,
    ),
    Inbox(Result<maka_protocol::session::SessionCatalogQueryResult, String>),
    Projects(Result<maka_protocol::project::QueryResult, String>),
    Connections(Result<Value, String>),
    Directory(
        pages::manage::directory::Request,
        Result<maka_protocol::project::QueryResult, String>,
    ),
    ChooseProject(
        pages::manage::choose_project::Request,
        Result<maka_protocol::project::QueryResult, String>,
    ),
    Locations(
        pages::manage::locations::Request,
        Result<maka_protocol::project::QueryResult, String>,
    ),
    Session(
        pages::sessions::DetailRequest,
        Result<Option<Box<maka_protocol::session::SessionCatalogProjection>>, String>,
    ),
    Models(pages::manage::models::Request, Result<Value, String>),
    EnabledModels(
        pages::manage::enabled_models::Request,
        Result<Value, String>,
    ),
    Credential(
        pages::manage::credentials::Request,
        Result<
            maka_protocol::configuration::CredentialVaultQueryResult,
            maka_client::RequestFailure,
        >,
    ),
    Onboard(
        pages::onboarding::Ticket,
        Result<pages::onboarding::ResultValue, maka_client::RequestFailure>,
    ),
}

pub async fn run<F, C, S, D>(options: Options, connect: F, shutdown: S) -> Result<(), Error>
where
    F: Fn(PathBuf) -> C,
    C: std::future::Future<Output = Result<(Client, mpsc::Receiver<Notification>), Error>>
        + Send
        + 'static,
    S: Fn(ShutdownRequest) -> D,
    D: std::future::Future<Output = Result<ShutdownOutcome, Error>> + Send + 'static,
{
    let i18n = i18n::I18n::from_environment(options.locale)?;
    let (_guard, mut screen) = terminal::Guard::enter(&i18n)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        terminal::restore();
        previous(info);
    }));
    let mut app = App::new(options.root, i18n);
    app.theme = theme::Theme::from_environment();
    let keep_locale = options.locale.is_some() || std::env::var_os("MAKA_LOCALE").is_some();
    let mut state = match state::State::open(&app.root, &options.profile).await {
        Ok(Some((state, saved))) => {
            if let Some(saved) = saved {
                saved.restore(&mut app, keep_locale)?;
            }
            Some(state)
        }
        Ok(None) => None,
        Err(error) => {
            return Err(app
                .i18n
                .format("state-open-failed", &[("error", &error.to_string())])
                .into());
        }
    };
    let mut input = EventStream::new();
    let mut jobs = JoinSet::new();
    // Local configuration I/O is not scoped to a Host connection epoch.
    let mut theme_jobs = JoinSet::new();
    let mut attachment_jobs = JoinSet::new();
    // Lookup owners close their finite Remote documents even when cancelled.
    let mut completion_jobs = JoinSet::new();
    let mut history_job = None;
    let mut client: Option<Client> = None;
    let mut notifications: Option<mpsc::Receiver<Notification>> = None;
    let mut notification_batch = Vec::with_capacity(32);
    let mut oauth_service: Option<maka_client::OAuthPresentationService> = None;
    let mut native_notifications: Option<maka_client::notifications::NativeNotificationService> =
        None;
    let (mut resource_runner, mut terminal_events) = pages::resources::terminal::Runner::new();
    let (mut watches, mut changes) = apps::io::Watches::new();
    let (mut transcript_runner, mut transcript_deliveries) = apps::io::transcript::Runner::new();
    let mut effect = app.apply(Action::Connect);
    let mut dirty = true;
    let mut flushed = false;
    let mut shutdown_target: Option<ShutdownRequest> = None;
    // Host disconnection is expected during shutdown; this task must outlive
    // the connection-scoped jobs and wait for actual storage/root release.
    let mut shutdown_job = None;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        app.reconcile_resources();
        if app.closing {
            app.completion_cancel();
        }
        if !app.closing && app.shutdown.prompt.is_none() {
            shutdown_target = None;
        }
        if app.closing && flushed && shutdown_job.is_none() {
            if let Some(target) = shutdown_target.clone() {
                app.shutdown.stopping = true;
                shutdown_job = Some(tokio::spawn(shutdown(target)));
                dirty = true;
            } else {
                break;
            }
        }
        if let Some(text) = app
            .resources
            .take_copy()
            .or_else(|| app.attention.take_copy())
        {
            let result = terminal::copy(&mut std::io::stdout(), &text);
            app.notice = Some(Notice::Clipboard {
                key: if result.is_ok() {
                    "chat-copy-requested"
                } else {
                    "chat-copy-failed"
                },
                until: std::time::Instant::now() + Duration::from_secs(3),
            });
            dirty = true;
        }
        if let Some(text) = app.apps_transcript_copy() {
            let result = terminal::copy(&mut std::io::stdout(), &text);
            app.notice = Some(Notice::Clipboard {
                key: if result.is_ok() {
                    "chat-copy-requested"
                } else {
                    "chat-copy-failed"
                },
                until: std::time::Instant::now() + Duration::from_secs(3),
            });
            dirty = true;
        }
        if let Some(key) = app.apps_transcript_notice() {
            app.notice = Some(Notice::Clipboard {
                key,
                until: std::time::Instant::now() + Duration::from_secs(3),
            });
            dirty = true;
        }
        app.advance_revision_uploads();
        if let Some(ticket) = app.clipboard_request() {
            let directory = state
                .as_ref()
                .map(|state| state.clipboard_directory.clone());
            attachment_jobs.spawn(pages::attachments::paste::read(ticket, directory));
            dirty = true;
        }
        if !app.closing
            && let Some(request) = app.attachment_browse_request()
        {
            attachment_jobs.spawn(async move {
                let result = pages::attachments::io::browse(&request).await;
                pages::attachments::Completed::Browsed(request, result)
            });
        }
        if let Some(request) = app.theme.request() {
            theme_jobs.spawn(async move {
                let result = request.execute().await;
                (request, result)
            });
        }
        if let Some(state) = &mut state
            && !(app.closing && flushed)
        {
            if app.closing && state.wait().is_some() {
                state.force(&mut app);
            }
            state.start(&mut app);
        }
        if let Some(client) = &client
            && !app.closing
        {
            if let Some((ticket, saved, transfer)) = app.attachment_read_request() {
                let client = client.clone();
                attachment_jobs.spawn(async move {
                    let result =
                        pages::attachments::io::prepare(&client, &ticket, saved, transfer).await;
                    pages::attachments::Completed::Prepared(ticket, result)
                });
                dirty = true;
            }
            if let Some(request) = app.revision_request() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_revision(request, &mut app);
                    } else {
                        app.revision_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::revision::execute(&client, &request).await;
                        Completed::Revised(request, result)
                    });
                }
                dirty = true;
            }
            if let Some(request) = app.bundle_request() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_bundle(request, &mut app);
                    } else {
                        app.bundle_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::bundle::execute(&client, &request).await;
                        Completed::Bundle(request, result)
                    });
                }
                dirty = true;
            }
            if let Some(request) = app.session_anchor_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = pages::session_controls::anchor::execute(&client, &request).await;
                    Completed::SessionAnchor(request, result)
                });
            }
            if let Some(request) = app.completion_request() {
                let client = client.clone();
                completion_jobs.spawn(async move {
                    let result = pages::completion::execute(&client, &request).await;
                    (request, result)
                });
            }
            if let Some(request) = app.resources.request() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_resources(request, &mut app);
                    } else {
                        app.resources_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::resources::execute(&client, &request).await;
                        Completed::Resources(request, result)
                    });
                }
                dirty = true;
            }
            if let Some(request) = app.session_controls_request() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_session_controls(request, &mut app);
                    } else {
                        app.session_controls_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::session_controls::execute(&client, &request).await;
                        Completed::SessionControls(request, result)
                    });
                }
                dirty = true;
            }
            if let Some(request) = app.resume_request() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_resume(request, &mut app);
                    } else {
                        app.resume_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::resume::execute(&client, &request).await;
                        Completed::Resumed(request, result)
                    });
                }
                dirty = true;
            }
            if let Some(request) = app.branch_request() {
                if request.query {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::branch::execute(&client, &request).await;
                        Completed::Branched(request, result)
                    });
                } else if let Some(state) = &mut state {
                    state.submit_branch(request, &mut app);
                } else {
                    app.branch_after_checkpoint(
                        &request,
                        &Err("TUI checkpoint unavailable".into()),
                    );
                }
                dirty = true;
            }
            if oauth_service.is_some() && native_notifications.is_some() {
                app.oauth_native_service_ready();
            }
            if oauth_service.is_some()
                && native_notifications.is_some()
                && let Some(request) = app.oauth_request()
            {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_oauth(request, &mut app);
                    } else {
                        app.oauth_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                    dirty = true;
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::manage::oauth::execute(&client, &request).await;
                        Completed::Oauth(Box::new(request), result)
                    });
                }
            }
            if !app.closing
                && let Some(generation) = app.providers.query()
            {
                let client = client.clone();
                jobs.spawn(async move {
                    Completed::Providers(
                        generation,
                        client
                            .provider_directory(maka_protocol::model_provider::Scope::Profile)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                });
            }
            let chat_route = match app.navigation.current() {
                navigation::Route::Session(ref id) if app.pending_new.contains_key(id) => {
                    navigation::Route::Workspace
                }
                route => route,
            };
            if let Some(id) = app.chat.select(&chat_route) {
                close_observation(&mut jobs, client.clone(), id);
            }
            if history_job.is_none()
                && let Some(request) = app.chat.history_request()
            {
                let client = client.clone();
                history_job = Some(
                    jobs.spawn(async move {
                        let result = pages::chat::history::execute(&client, &request)
                            .await
                            .map_err(|error| error.to_string());
                        Completed::History(request, result)
                    })
                    .id(),
                );
            }
            if app.chat.error.is_some()
                && let Some(id) = app.chat.subscription.take()
            {
                close_observation(&mut jobs, client.clone(), id);
            }
            if let Some(request) = app.chat.open_query() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = pages::chat::open(&client, &request).await.map(Box::new);
                    Completed::ChatOpened(request, result)
                });
            }
            if let Some(request) = app.chat.context_query() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = async {
                        let value = client
                            .request(
                                Operation::ContextDiagnosticsQuery,
                                json!({"sessionId":request.session}),
                            )
                            .await?;
                        Ok::<_, Error>(maka_protocol::context::decode_context_diagnostics_result(
                            &value,
                        )?)
                    }
                    .await
                    .map_err(|error| error.to_string());
                    Completed::Context(request, result)
                });
            }
            if let Some(input) = app.sessions.query() {
                let client = client.clone();
                jobs.spawn(async move {
                    Completed::Catalog(
                        client
                            .session_catalog(input)
                            .await
                            .map_err(|e| e.to_string()),
                    )
                });
            }
            if app.palette.is_some()
                && let ConnectionState::Connected { root_id, epoch } = &app.connection
                && let Some(request) = app.command_palette.session_request(root_id, epoch)
            {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client
                        .session_catalog(request.query.clone())
                        .await
                        .map_err(|error| error.to_string());
                    Completed::PaletteSessions(request, result)
                });
            }
            if let Some(input) = app.inbox.query() {
                let client = client.clone();
                jobs.spawn(async move {
                    Completed::Inbox(
                        client
                            .session_catalog(input)
                            .await
                            .map_err(|e| e.to_string()),
                    )
                });
            }
            // Always loaded: the sidebar labels project workspaces by name.
            if let Some(input) = app.projects.query() {
                let client = client.clone();
                jobs.spawn(async move {
                    Completed::Projects(
                        client
                            .project_catalog(input)
                            .await
                            .map_err(|e| e.to_string()),
                    )
                });
            }
            if (app.navigation.current() == navigation::Route::Connections
                || app.connections.setup() == pages::connections::Setup::Loading)
                && let Some(input) = app.connections.query()
            {
                let client = client.clone();
                jobs.spawn(async move {
                    Completed::Connections(
                        client
                            .connection_catalog(input)
                            .await
                            .map_err(|e| e.to_string()),
                    )
                });
            }
            if let Some(request) = app.sessions.detail_query() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client.session(&request.id).await.map_err(|e| e.to_string());
                    Completed::Session(request, result)
                });
            }
            watches.reconcile_documents(app.apps_executions());
            watches.reconcile(client, app.apps_watches());
            if transcript_runner
                .reconcile(client, watches.transcripts(app.apps_transcript_mounts()))
                .is_err()
            {
                app.apps_transcript_failed();
                dirty = true;
            }
            app.apps_transcript_pages(&transcript_runner);
            if let Some(request) = app.plugins_request() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_plugins(request, &mut app);
                    } else {
                        app.plugins_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = pages::plugins::execute(&client, &request).await;
                        Completed::Plugins(request, result)
                    });
                }
                dirty = true;
            }
            for request in app.apps_requests() {
                if request.needs_checkpoint() {
                    if let Some(state) = &mut state {
                        state.submit_app(request, &mut app);
                    } else {
                        app.apps_after_checkpoint(
                            &request,
                            &Err("TUI checkpoint unavailable".into()),
                        );
                    }
                } else {
                    let document = watches.document(client, &request);
                    let client = client.clone();
                    jobs.spawn(async move {
                        let result = apps::execute(&client, &request, document).await;
                        Completed::Extension(Box::new(request), result)
                    });
                }
                dirty = true;
            }
            if let Some(request) = app.directory_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client
                        .project_catalog(request.query.clone())
                        .await
                        .map_err(|e| e.to_string());
                    Completed::Directory(request, result)
                });
            }
            if let Some(request) = app.choose_project_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client
                        .project_catalog(request.query.clone())
                        .await
                        .map_err(|e| e.to_string());
                    Completed::ChooseProject(request, result)
                });
            }
            if let Some(request) = app.locations_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client
                        .project_catalog(request.query.clone())
                        .await
                        .map_err(|e| e.to_string());
                    Completed::Locations(request, result)
                });
            }
            if let Some(request) = app.models_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client
                        .connection_catalog(request.query.clone())
                        .await
                        .map_err(|e| e.to_string());
                    Completed::Models(request, result)
                });
            }
            if let Some(request) = app.enabled_models_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client
                        .connection_catalog(request.query.clone())
                        .await
                        .map_err(|e| e.to_string());
                    Completed::EnabledModels(request, result)
                });
            }
            if let Some(request) = app.credential_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = client.credential_status(request.locator()).await;
                    Completed::Credential(request, result)
                });
            }
            if let Some(request) = app.preferences_request() {
                let ticket = request.ticket();
                let client = client.clone();
                jobs.spawn(async move {
                    let result = pages::manage::preferences::execute(&client, &request).await;
                    Completed::Preferences(ticket, result)
                });
            }
            if let Some(request) = app.sandbox_defaults_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = pages::manage::sandbox::defaults::read(&client).await;
                    Completed::SandboxDefaults(request, result)
                });
            }
            if let Some(request) = app.removal_request() {
                let client = client.clone();
                jobs.spawn(async move {
                    let result = pages::manage::removal::read(&client, &request).await;
                    Completed::Removal(request, result)
                });
            }
        }
        if dirty {
            app.oauth_before_draw();
            app.sync_interaction();
            terminal::draw(&mut screen, |frame| view::draw(frame, &mut app))?;
            app.oauth_after_draw();
            app.attention_frame_committed();
        }
        if let Some(client) = &client {
            app.sync_resource_terminal(client, &mut resource_runner);
        }
        // Paging can depend on the just-measured viewport. Dispatch before
        // waiting for input, including when motion is disabled and the app is idle.
        if let Some(client) = &client
            && !app.closing
            && let Some(request) = app.chat.page_query()
        {
            let client = client.clone();
            jobs.spawn(async move {
                let result = async {
                    let page = client.transcript_page(request.input.clone()).await?;
                    pages::chat::complete_page(&client, &request.input.subscription_id, page).await
                }
                .await
                .map_err(|e: Error| e.to_string());
                Completed::ChatPage(request, result)
            });
        }
        if let Some(action) = effect.take() {
            match action {
                Action::Manage(pages::manage::Command::Oauth(command)) => {
                    app.oauth_copy(command);
                    dirty = true;
                    continue;
                }
                Action::Onboard(
                    command @ (pages::onboarding::Command::Verify
                    | pages::onboarding::Command::Save),
                ) => {
                    if let Some(client) = client.clone()
                        && let Some(request) =
                            app.onboarding_request(command == pages::onboarding::Command::Save)
                    {
                        jobs.spawn(async move {
                            let ticket = request.ticket.clone();
                            let result = pages::onboarding::execute(&client, request).await;
                            Completed::Onboard(ticket, result)
                        });
                    }
                    dirty = true;
                    continue;
                }
                Action::Manage(pages::manage::Command::Save) => {
                    if app.save_draft_model() {
                        dirty = true;
                        continue;
                    }
                    if let Some(client) = client.clone()
                        && let Some(ticket) = app.management_request()
                    {
                        jobs.spawn(async move {
                            let result = pages::manage::execute(&client, &ticket).await;
                            Completed::Managed(Box::new(ticket), result)
                        });
                    }
                    dirty = true;
                    continue;
                }
                Action::EditComposer(target) => {
                    if target.command == pages::actions::native::Edit::Cut {
                        if let Some(text) = target.text(&app) {
                            jobs.spawn(async move {
                                let result = tokio::task::spawn_blocking(move || {
                                    terminal::copy_native(text)
                                })
                                .await
                                .unwrap_or(Err("chat-copy-failed"));
                                Completed::ComposerCut(target, result)
                            });
                        }
                        dirty = true;
                        continue;
                    }
                    let result = target.text(&app).ok_or("chat-copy-empty").and_then(|text| {
                        terminal::copy(&mut std::io::stdout(), &text)
                            .map_err(|_| "chat-copy-failed")
                    });
                    app.notice = Some(Notice::Clipboard {
                        key: result.map_or_else(|key| key, |_| "chat-copy-requested"),
                        until: std::time::Instant::now() + Duration::from_secs(3),
                    });
                    dirty = true;
                    continue;
                }
                Action::CopyFile(path) => {
                    let result = terminal::copy(&mut std::io::stdout(), &path);
                    app.notice = Some(Notice::Clipboard {
                        key: if result.is_ok() {
                            "chat-copy-requested"
                        } else {
                            "chat-copy-failed"
                        },
                        until: std::time::Instant::now() + Duration::from_secs(3),
                    });
                    dirty = true;
                    continue;
                }
                Action::Copy(_) | Action::CopyMessage { .. } | Action::CopyReply { .. } => {
                    let text = match action {
                        Action::Copy(mode) => app
                            .chat
                            .reader()
                            .ok_or("chat-copy-empty")
                            .and_then(|reader| reader.copy_text(mode, app.chrome.ascii)),
                        Action::CopyMessage { target, mode } => target.text(&app, mode),
                        Action::CopyReply { key, revision } => app
                            .chat
                            .reader()
                            .ok_or("chat-copy-empty")
                            .and_then(|reader| reader.copy_source(&key, &revision)),
                        _ => unreachable!(),
                    };
                    let result = text.and_then(|text| {
                        terminal::copy(&mut std::io::stdout(), &text)
                            .map_err(|_| "chat-copy-failed")
                    });
                    app.notice = Some(Notice::Clipboard {
                        key: result.map_or_else(|key| key, |_| "chat-copy-requested"),
                        until: std::time::Instant::now() + Duration::from_secs(3),
                    });
                    dirty = true;
                    continue;
                }
                Action::Interaction(command) => {
                    if let Some((ticket, answer)) = app.interaction_request(command)
                        && let Some(client) = client.clone()
                    {
                        jobs.spawn(async move {
                            let result = if let Some(answer) = answer {
                                client.answer_interaction(&ticket.snapshot, answer).await
                            } else {
                                client.interaction(&ticket.snapshot).await
                            };
                            Completed::Interaction(Box::new((ticket, result)))
                        });
                    }
                    dirty = true;
                    continue;
                }
                Action::Quit | Action::Detach | Action::ConfirmQuit => {
                    match action {
                        Action::Quit => {
                            shutdown_target = client.as_ref().map(|client| ShutdownRequest {
                                root: app.root.clone(),
                                identity: client.identity.clone(),
                                interrupt: false,
                            });
                        }
                        Action::ConfirmQuit => {
                            if !matches!(app.shutdown.prompt, Some(shutdown::Prompt::Busy)) {
                                continue;
                            }
                            if let Some(target) = &mut shutdown_target {
                                target.interrupt = true;
                            }
                        }
                        Action::Detach => shutdown_target = None,
                        _ => unreachable!(),
                    }
                    app.shutdown = shutdown::State::default();
                    flushed = false;
                    app.attachments.disconnect();
                    app.apps.disconnect();
                    watches.stop();
                    transcript_runner.stop();
                    resource_runner.stop();
                    app.resources.disconnect();
                    app.completion.disconnect();
                    app.attention.disconnect();
                    app.bundle.disconnect();
                    app.session_controls.disconnect();
                    app.plugins.disconnect();
                    app.resume.disconnect();
                    app.branch.disconnect();
                    app.revision.disconnect();
                    if let Some(state) = &mut state {
                        app.oauth_abandon_checkpoint();
                        for request in state.cancel_requests() {
                            app.abandon_checkpoint(&request);
                        }
                        state.force(&mut app);
                        app.closing = true;
                        dirty = true;
                        continue;
                    }
                    app.closing = true;
                    flushed = true;
                    continue;
                }
                Action::Connect => {
                    app.branch.disconnect();
                    app.resume.disconnect();
                    app.revision.disconnect();
                    if let Some(state) = &mut state {
                        state.cancel_requests();
                    }
                    app.queue.abandon();
                    app.abandon_management();
                    app.management.oauth.abandon();
                    app.abandon_onboarding();
                    app.abandon_interaction();
                    app.abandon_pending_submissions();
                    app.attachments.disconnect();
                    app.apps.disconnect();
                    watches.stop();
                    transcript_runner.stop();
                    resource_runner.stop();
                    app.resources.disconnect();
                    app.completion.disconnect();
                    app.attention.disconnect();
                    app.bundle.disconnect();
                    app.session_controls.disconnect();
                    app.plugins.disconnect();
                    app.resume.disconnect();
                    app.creating = false;
                    jobs = JoinSet::new();
                    history_job = None;
                    app.chat.reset();
                    if let Some(old) = client.take() {
                        old.disconnect();
                    }
                    notifications = None;
                    oauth_service = None;
                    native_notifications = None;
                    app.refreshing = false;
                    let connection = connect(app.root.clone());
                    jobs.spawn(async move { Completed::Connected(connection.await) });
                }
                Action::StopTurn(target) => {
                    if let Some(client) = client.clone()
                        && client.identity.root_id == target.root
                        && client.identity.host_epoch == target.epoch
                        && app.stop_target().as_ref() == Some(&target)
                        && app.chat.start_stop(&target)
                    {
                        jobs.spawn(async move {
                            let result = client.stop_turn(target.input()).await;
                            Completed::Stopped(target, result)
                        });
                    }
                    dirty = true;
                    continue;
                }
                Action::Queue(command) => {
                    if let Some(client) = client.clone()
                        && let Some(ticket) = app.queue_request(command)
                    {
                        jobs.spawn(async move {
                            let result = pages::queue::execute(&client, &ticket).await;
                            Completed::Queue(ticket, result)
                        });
                    }
                    dirty = true;
                    continue;
                }
                Action::SendMessage | Action::SteerMessage | Action::RetrySubmission => {
                    if client.is_some()
                        && let Some(request) = if action == Action::RetrySubmission {
                            app.retry_submission()
                        } else if action == Action::SteerMessage {
                            app.submission_for(maka_protocol::message::Placement::CurrentTurn)
                        } else {
                            app.submission()
                        }
                    {
                        if let Some(state) = &mut state {
                            state.submit(request, &mut app);
                        } else {
                            let error = app.i18n.text("state-unavailable");
                            app.after_checkpoint(&request, &Err(error.clone()));
                            app.state_error = Some(error);
                        }
                    }
                    dirty = true;
                    continue;
                }
                Action::ReconcileSubmission => {
                    if let Some(client) = client.clone()
                        && let Some(request) = app.reconciliation()
                    {
                        let pending_create = app.pending_new.contains_key(&request.session);
                        jobs.spawn(async move {
                            if pending_create {
                                return match client.session(&request.session).await {
                                    Ok(Some(_)) => Completed::Reconciled(
                                        request.clone(),
                                        client
                                            .message_execution(&request.session, &request.id)
                                            .await,
                                        Some(true),
                                    ),
                                    Ok(None) => {
                                        Completed::Reconciled(request, Ok(None), Some(false))
                                    }
                                    Err(error) => Completed::Reconciled(request, Err(error), None),
                                };
                            }
                            let result = client
                                .message_execution(&request.session, &request.id)
                                .await;
                            Completed::Reconciled(request, result, None)
                        });
                    }
                    dirty = true;
                    continue;
                }
                Action::CreateSession | Action::Project(pages::projects::Command::Create(_)) => {
                    if action == Action::CreateSession
                        && let navigation::Route::Session(id) = app.navigation.current()
                        && app.pending_new.contains_key(&id)
                        && app
                            .drafts
                            .get(&id)
                            .is_some_and(|draft| draft.text().is_empty())
                        && !app.attachments.has(&id)
                        && !app.has_directories(&id)
                        && !app.has_selections(&id)
                        && !app.completion_requires_idle(&id)
                    {
                        app.creating = false;
                        app.focus = app::Focus::Composer;
                        dirty = true;
                        continue;
                    }
                    if action == Action::CreateSession
                        && let Some(id) = app
                            .pending_new
                            .keys()
                            .filter(|id| !app.tabs.contains(id))
                            .min()
                            .cloned()
                    {
                        app.creating = false;
                        app.apply(Action::Visit(navigation::Route::Session(id)));
                        dirty = true;
                        continue;
                    }
                    let input = (|| -> Result<_, Error> {
                        let workspace = match action {
                            Action::Project(pages::projects::Command::Create(id)) => {
                                json!({"kind":"project", "projectId":id})
                            }
                            _ => json!({"kind":"host_path","path":std::env::current_dir()?}),
                        };
                        Ok(maka_protocol::session::decode_session_create_input(
                            &json!({
                                "sessionId":uuid::Uuid::new_v4().to_string(),
                                "workspace":workspace,
                                "modelTarget":{"kind":"default"}
                            }),
                        )?)
                    })();
                    app.creating = false;
                    match input {
                        Ok(input) => {
                            let id = input.session_id.clone();
                            app.pending_new.insert(id.clone(), input);
                            app.apply(Action::Visit(navigation::Route::Session(id.clone())));
                            if app.navigation.current() != navigation::Route::Session(id.clone()) {
                                app.pending_new.remove(&id);
                            }
                        }
                        Err(error) => app.notice = Some(Notice::CreateFailed(error.to_string())),
                    }
                    dirty = true;
                    continue;
                }
                Action::Refresh => {
                    if let Some(client) = client.clone() {
                        jobs.spawn(async move {
                            Completed::Status(
                                client
                                    .request(Operation::HostStatus, json!({}))
                                    .await
                                    .map_err(|e| e.to_string()),
                            )
                        });
                    }
                }
                Action::RefreshSession => {
                    if let Some(id) = app.chat.refresh()
                        && let Some(client) = client.clone()
                    {
                        close_observation(&mut jobs, client, id);
                    }
                    dirty = true;
                    continue;
                }
                _ => {}
            }
        }
        let state_wait = state.as_ref().and_then(state::State::wait);
        let oauth_wait = app.oauth_wait();
        let chat_deadline = if client.is_some() && !app.closing {
            app.chat.open_deadline()
        } else {
            None
        };
        // Notifications and completions can change persisted owners even when
        // they do not schedule a disk write. Only classified local input and
        // transient reader/paint events retain the derived admission cache.
        let mut checkpoint_impact = state::Impact::Other;
        tokio::select! {
            _ = pages::chat::wait_for_open(chat_deadline) => {
                checkpoint_impact = state::Impact::Reading;
            }
            result = async {
                match &mut shutdown_job {
                    Some(job) => job.await,
                    None => std::future::pending().await,
                }
            } => {
                shutdown_job = None;
                app.closing = false;
                match result {
                    Ok(Ok(ShutdownOutcome::Stopped)) => break,
                    Ok(Ok(ShutdownOutcome::Busy)) => app.shutdown.show(shutdown::Prompt::Busy),
                    Ok(Err(error)) => app.shutdown.show(shutdown::Prompt::Failed(error.to_string())),
                    Err(error) => app.shutdown.show(shutdown::Prompt::Failed(error.to_string())),
                }
                dirty = true;
            }
            _ = async {
                match oauth_wait {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            } => { checkpoint_impact = state::Impact::Reading; }
            completed = completion_jobs.join_next(), if !completion_jobs.is_empty() => {
                match completed {
                    Some(Ok((request, result))) => app.completion_completed(request, result),
                    Some(Err(_)) => {
                        if let Some(client) = &client { client.disconnect(); }
                    }
                    None => {}
                }
                dirty = true;
            }
            request = async {
                match oauth_service.as_mut() {
                    Some(service) => service.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(request) = request { app.oauth_presentation(request); }
                else {
                    if let Some(client) = &client { client.disconnect(); }
                    oauth_service = None;
                    app.oauth_service_closed();
                }
                dirty = true;
            }
            delivery = async {
                match native_notifications.as_mut() {
                    Some(service) => service.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match delivery {
                    Some(delivery) if !app.closing => {
                        if let Some(client) = &client {
                            dirty |= app.attention.receive(&client.identity.root_id, delivery);
                        }
                    }
                    Some(_) => {}
                    None => {
                        if let Some(client) = &client { client.disconnect(); }
                        native_notifications = None;
                        app.attention.disconnect();
                    }
                }
                checkpoint_impact = state::Impact::Reading;
            }
            event = terminal_events.recv() => {
                if let Some(event) = event { dirty |= app.resources.terminal_event(event); }
                checkpoint_impact = state::Impact::Reading;
            }
            _ = async {
                match state_wait {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            } => {}
            written = async {
                match &mut state {
                    Some(state) => state.completed().await,
                    None => std::future::pending().await,
                }
            } => {
                // Completing an autosave changes no owner. Keep the sizes from
                // its capture plus any field deltas typed while it was writing.
                if written.requests.is_empty() && written.apps.is_empty()
                    && written.oauth.is_none() && written.branch.is_none()
                    && written.bundle.is_none() && written.session_controls.is_none() && written.resources.is_none() && written.plugins.is_none()
                    && written.resume.is_none() && written.revision.is_none()
                    && written.attachment.is_none() {
                    checkpoint_impact = state::Impact::Reading;
                }
                if let Some(request) = written.plugins
                    && app.plugins_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move { let result = pages::plugins::execute(&client, &request).await; Completed::Plugins(request, result) });
                }
                if let Some(request) = written.bundle
                    && app.bundle_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move {
                        let result=pages::bundle::execute(&client,&request).await;
                        Completed::Bundle(request,result)
                    });
                }
                if let Some(request) = written.resources
                    && app.resources_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move { let result = pages::resources::execute(&client, &request).await; Completed::Resources(request, result) });
                }
                if let Some(request) = written.session_controls
                    && app.session_controls_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move {
                        let result=pages::session_controls::execute(&client,&request).await;
                        Completed::SessionControls(request,result)
                    });
                }
                if let Some(request) = written.resume
                    && app.resume_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move {
                        let result = pages::resume::execute(&client, &request).await;
                        Completed::Resumed(request, result)
                    });
                }
                for request in written.apps {
                    if app.apps_after_checkpoint(&request, &written.result)
                        && let Some(client) = client.clone() {
                        let document = watches.document(&client, &request);
                        jobs.spawn(async move {
                            let result = apps::execute(&client, &request, document).await;
                            Completed::Extension(Box::new(request), result)
                        });
                    }
                }
                if let Some(ticket) = written.attachment
                    && let Some((prepared, transfer)) = app.attachment_after_checkpoint(&ticket, &written.result)
                    && let Some(client) = client.clone() {
                    attachment_jobs.spawn(async move {
                        let result = pages::attachments::io::upload(&client, &ticket, prepared, transfer).await;
                        pages::attachments::Completed::Uploaded(ticket, result)
                    });
                }
                if let Some(request) = written.revision
                    && app.revision_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move {
                        let result = pages::revision::execute(&client, &request).await;
                        Completed::Revised(request, result)
                    });
                }
                if let Some(request) = written.branch
                    && app.branch_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move {
                        let result = pages::branch::execute(&client, &request).await;
                        Completed::Branched(request, result)
                    });
                }
                if let Some(request) = written.oauth
                    && app.oauth_after_checkpoint(&request, &written.result)
                    && let Some(client) = client.clone() {
                    jobs.spawn(async move {
                        let result = pages::manage::oauth::execute(&client, &request).await;
                        Completed::Oauth(Box::new(request), result)
                    });
                }
                for request in written.requests {
                    if app.after_checkpoint(&request, &written.result)
                        && let Some(client) = client.clone() {
                        let create = app.pending_new.get(&request.session).cloned();
                        jobs.spawn(async move {
                            let mut created = false;
                            if let Some(input) = create {
                                match client.create_session(input).await {
                                    Ok(_) => created = true,
                                    Err(maka_client::RequestFailure::Rejected(
                                        maka_client::ClientError::Rejected(error),
                                    )) if matches!(
                                        error.code,
                                        maka_protocol::OperationErrorCode::OutcomeUnknown
                                            | maka_protocol::OperationErrorCode::CommitOutcomeUnknown
                                    ) => {
                                        return Completed::Submitted(
                                            request,
                                            Err(maka_client::RequestFailure::Unknown(
                                                maka_client::ClientError::Rejected(error),
                                            )),
                                            false,
                                        );
                                    }
                                    Err(error) => return Completed::Submitted(request, Err(error), false),
                                }
                            }
                            let result = client.submit_message(request.input()).await;
                            Completed::Submitted(request, result, created)
                        });
                    }
                }
                app.state_error = written.result.err();
                if app.state_error.is_some() { app.closing = false; }
                if app.closing && state.as_ref().is_some_and(state::State::idle) {
                    flushed = true;
                }
                dirty = true;
            }
            _ = async {
                match app.selection_wait(std::time::Instant::now()) {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            } => {
                dirty = app.selection_scroll(std::time::Instant::now());
                if dirty && let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
            }
            _ = async {
                match &app.notice {
                    Some(Notice::Clipboard { until, .. }) => tokio::time::sleep(until.saturating_duration_since(std::time::Instant::now())).await,
                    _ => std::future::pending().await,
                }
            } => { checkpoint_impact = state::Impact::Reading; app.notice = None; dirty = true; }
            _ = async {
                let wait = app.chat.history.as_ref()
                    .and_then(|history| history.wait())
                    .filter(|_| history_job.is_none() && client.is_some() && app.chat.error.is_none() && app.chat.snapshot.is_some());
                match wait { Some(wait) => tokio::time::sleep(wait).await, None => std::future::pending().await }
            } => {}
            _ = async {
                match app.chat.context.wait().filter(|_| client.is_some() && app.chat.error.is_none() && app.chat.snapshot.is_some()) {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            } => {}
            _ = async {
                match app.tooltip_wait() {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending::<()>().await,
                }
            } => { checkpoint_impact = state::Impact::Reading; dirty = true; }
            _ = async {
                let now=std::time::Instant::now();
                let wait=if app.chrome.animating() { Some(Duration::from_millis(16)) }
                    else { app.chrome.animation.wait(now) };
                match wait { Some(wait)=>tokio::time::sleep(wait).await, None=>std::future::pending().await }
            } => {
                checkpoint_impact = state::Impact::Reading;
                dirty = true;
            }
            event = input.next() => {
                match event {
                    Some(Ok(event)) => {
                        (dirty, effect) = app.input(event);
                        checkpoint_impact = app.checkpoint_input_impact();
                        if dirty && let Some(state) = &mut state { state.changed(&mut app, checkpoint_impact); }
                    }
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)), if app.attachments.uploading() && app.chrome.window_focused => { checkpoint_impact = state::Impact::Reading; dirty = true; }
            completed = attachment_jobs.join_next(), if !attachment_jobs.is_empty() => {
                match completed {
                    Some(Ok(pages::attachments::Completed::Pasted(ticket, result))) => app.clipboard_pasted(ticket, result),
                    Some(Ok(pages::attachments::Completed::Browsed(request, result))) => app.attachment_browsed(request, result),
                    Some(Ok(pages::attachments::Completed::Prepared(ticket, result))) => {
                        if let Some(ticket) = app.attachment_prepared(ticket, result) {
                            if let Some(state) = &mut state { state.submit_attachment(ticket, &mut app); }
                            else { app.attachment_after_checkpoint(&ticket, &Err("TUI checkpoint unavailable".into())); }
                        }
                    }
                    Some(Ok(pages::attachments::Completed::Uploaded(ticket, result))) => app.attachment_uploaded(ticket, result),
                    Some(Err(error)) => return Err(error.into()),
                    None => {}
                }
                if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                dirty = true;
            }
            completed = theme_jobs.join_next(), if !theme_jobs.is_empty() => {
                if let Some(completed) = completed {
                    let (request, result) = completed?;
                    app.theme.complete(request, result);
                    if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    dirty = true;
                }
            }
            completed = jobs.join_next(), if !jobs.is_empty() => {
                match completed {
                    Some(Ok(Completed::Revised(request, result))) => {
                        app.revision_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Branched(request, result))) => {
                        app.branch_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Oauth(request, result))) => {
                        if let Some(service) = app.oauth_completed(*request, result) {
                            oauth_service = Some(service);
                        }
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Plugins(request, result))) => {
                        app.plugins_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Preferences(ticket, result))) => {
                        app.preferences_completed(ticket, result);
                    }
                    Some(Ok(Completed::Resources(request, result))) => {
                        app.resources.complete(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::NativeServices(identity, result))) => {
                        if client.as_ref().is_some_and(|client| client.identity == identity) && !app.closing {
                            match result {
                                Ok(services) => {
                                    oauth_service = Some(services.oauth);
                                    native_notifications = Some(services.notifications);
                                    app.oauth_native_service_ready();
                                }
                                Err(error) => {
                                    app.notice = Some(Notice::Diagnostic(error.to_string()));
                                    if let Some(client) = &client { client.disconnect(); }
                                }
                            }
                        }
                    }
                    Some(Ok(Completed::SessionControls(request, result))) => {
                        app.session_controls_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::SessionAnchor(request, result))) => {
                        app.session_anchor_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Bundle(request, result))) => {
                        app.bundle_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Resumed(request, result))) => {
                        app.resume_completed(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Managed(ticket, result))) => {
                        app.management_completed(*ticket, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    },
                    Some(Ok(Completed::Removal(request, result))) => {
                        app.removal_read(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    },
                    Some(Ok(Completed::Directory(request, result))) => app.directory_completed(request, result),
                    Some(Ok(Completed::Extension(request, result))) => {
                        app.apps_complete(*request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::ChooseProject(request, result))) => app.choose_project_completed(request, result),
                    Some(Ok(Completed::Locations(request,result))) => app.locations_completed(request,result),
                    Some(Ok(Completed::Models(request,result)))=>app.models_completed(request,result),
                    Some(Ok(Completed::EnabledModels(request,result)))=>app.enabled_models_completed(request,result),
                    Some(Ok(Completed::Credential(request,result)))=>app.credential_completed(request,result),
                    Some(Ok(Completed::SandboxDefaults(request,result)))=>app.sandbox_defaults_completed(request,result),
                    Some(Ok(Completed::Onboard(ticket,result)))=>app.onboarding_completed(ticket,result),
                    Some(Ok(Completed::ComposerCut(target, result))) => {
                        if target.cut_completed(&mut app, result)
                            && let Some(state) = &mut state {
                            state.changed(&mut app, state::Impact::Other);
                        }
                    }
                    Some(Ok(Completed::History(request, result))) => {
                        history_job = None;
                        app.chat.history_completed(request, result, &app.i18n, app.chrome.ascii);
                    }
                    Some(Ok(Completed::Queue(ticket, result))) => app.queue_completed(ticket, result),
                    Some(Ok(Completed::Stopped(target, result))) => app.chat.stopped(target, result),
                    Some(Ok(Completed::Context(request, result))) => app.chat.context_completed(request, result),
                    Some(Ok(Completed::Interaction(result))) => {
                        let (ticket, result) = *result;
                        app.interaction_completed(ticket, result);
                    }
                    Some(Ok(Completed::Reconciled(request, result, creation_exists))) => {
                        if !creation_exists.is_some_and(|exists| {
                            app.reconciled_pending_creation(&request, exists)
                        }) {
                            app.reconciled(request, result);
                        }
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::Submitted(request, result, created))) => {
                        let definite_create_failure = matches!(
                            &result,
                            Err(maka_client::RequestFailure::NotDispatched(_))
                        ) || matches!(
                            &result,
                            Err(maka_client::RequestFailure::Rejected(
                                maka_client::ClientError::Rejected(error)
                            )) if !matches!(error.code,
                                maka_protocol::OperationErrorCode::OutcomeUnknown
                                    | maka_protocol::OperationErrorCode::CommitOutcomeUnknown)
                        );
                        let rejected_create = !created
                            && app.pending_new.contains_key(&request.session)
                            && definite_create_failure
                            && app.sending.get(&request.session).is_some_and(|sent| {
                                matches!(sent.delivery, pages::sending::Delivery::Pending)
                            });
                        let create_error = if rejected_create {
                            result.as_ref().err().map(ToString::to_string)
                        } else {
                            None
                        };
                        if created && app.pending_new.remove(&request.session).is_some() {
                            if matches!(app.notice, Some(Notice::CreateFailed(_))) {
                                app.notice = None;
                            }
                            app.sessions.refresh();
                            if app.navigation.current() == navigation::Route::Session(request.session.clone()) {
                                app.sessions.open(&request.session);
                            }
                        }
                        let session = request.session.clone();
                        app.submitted(request, result);
                        if let Some(error) = create_error {
                            app.sending.remove(&session);
                            app.notice = Some(Notice::CreateFailed(error));
                        }
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::ChatOpened(request, result))) => {
                        match app.chat.opened(request, result.map(|opened| *opened)) {
                            pages::chat::OpenEffect::Close(id) => {
                                if let Some(client) = client.clone() { close_observation(&mut jobs, client, id); }
                            }
                            pages::chat::OpenEffect::Ready(id) => {
                                if let Some(client) = client.clone() {
                                    jobs.spawn(async move {
                                        let result = client.ready_subscription(&id).await.map_err(|e| e.to_string());
                                        Completed::ChatReady(id, result)
                                    });
                                }
                            }
                            pages::chat::OpenEffect::None => {}
                        }
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::ChatPage(request, result))) => {
                        app.chat.page(request, result);
                        if let Some(state) = &mut state { state.changed(&mut app, state::Impact::Other); }
                    }
                    Some(Ok(Completed::ChatReady(id, Err(error)))) if app.chat.subscription.as_ref() == Some(&id) => app.chat.error = Some(error),
                    Some(Ok(Completed::Connected(Ok((connected, receiver))))) => {
                        if app.closing { connected.disconnect(); continue; }
                        if state.is_none() {
                            match state::State::open(&app.root, &options.profile).await {
                                Ok(Some((opened, saved))) => {
                                    if let Some(saved) = saved { saved.restore(&mut app, keep_locale)?; }
                                    state = Some(opened);
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    connected.disconnect();
                                    return Err(app.i18n.format("state-open-failed", &[("error", &error.to_string())]).into());
                                }
                            }
                        }
                        if !app.bind_root(&connected.identity.root_id) {
                            app.attention.clear();
                            connected.disconnect();
                            app.connection = ConnectionState::Failed(app.i18n.text("host-root-changed"));
                            dirty = true;
                            continue;
                        }
                        app.connection = ConnectionState::Connected {
                            root_id: connected.identity.root_id.clone(),
                            epoch: connected.identity.host_epoch.clone(),
                        };
                        let service_client = connected.clone();
                        let identity = connected.identity.clone();
                        jobs.spawn(async move { Completed::NativeServices(identity, service_client.publish_native_services().await) });
                        client = Some(connected);
                        notifications = Some(receiver);
                        app.sessions.refresh();
                        app.inbox.refresh();
                        app.projects.refresh();
                        app.connections.refresh();
                        app.providers.refresh();
                        if let navigation::Route::Session(id) = app.navigation.current()
                            && !app.pending_new.contains_key(&id) { app.sessions.open(&id); }
                        effect = app.apply(Action::Refresh);
                    }
                    Some(Ok(Completed::Connected(Err(error)))) => app.connection = ConnectionState::Failed(error.to_string()),
                    Some(Ok(Completed::Catalog(result))) => app.sessions.complete(result),
                    Some(Ok(Completed::PaletteSessions(request, result))) => {
                        if app.palette.is_some() {
                            let host = match &app.connection {
                                ConnectionState::Connected { root_id, epoch } => {
                                    Some((root_id.as_str(), epoch.as_str()))
                                }
                                _ => None,
                            };
                            if app.command_palette.session_completed(&request, result, host) {
                                let last = app.commands().len().saturating_sub(1);
                                if let Some(selected) = app.palette.as_mut() {
                                    *selected = (*selected).min(last);
                                }
                                app.layer.retire();
                            }
                        }
                    },
                    Some(Ok(Completed::Inbox(result))) => app.inbox.complete(result),
                    Some(Ok(Completed::Projects(result))) => app.projects.complete(result),
                    Some(Ok(Completed::Connections(result))) => {
                        app.connections.complete(result);
                        if matches!(app.notice, Some(Notice::Local("connections-loading" | "connections-failed"))) {
                            app.notice = match app.connections.setup() {
                                pages::connections::Setup::Loading => Some(Notice::Local("connections-loading")),
                                pages::connections::Setup::Failed => Some(Notice::Local("connections-failed")),
                                _ => None,
                            };
                        }
                        app.return_from_model_setup();
                    },
                    Some(Ok(Completed::Providers(generation, result))) => {
                        app.providers.complete(generation, result);
                        app.provider_commands_loaded();
                        app.oauth_catalog_loaded();
                        app.onboarding_catalog_loaded();
                        if app.providers.failed() {
                            app.notice = Some(Notice::Local("providers-failed"));
                        }
                    },
                    Some(Ok(Completed::Session(request, result))) => {
                        app.sessions.complete_detail(request, result);
                        if let pages::sessions::Detail::Missing { id } = &app.sessions.detail {
                            let id = id.clone();
                            app.session_removed(&id, false);
                        }
                    },
                    Some(Ok(Completed::Status(result))) => {
                        app.refreshing = false;
                        match result {
                            Ok(status) if client.as_ref().is_some_and(|c| status["hostEpoch"] == c.identity.host_epoch) => app.status = Some(status),
                            Ok(_) => {
                                if let Some(old) = client.take() { old.disconnect(); }
                                app.chat.disconnect(app.i18n.text("host-wrong-epoch"));
                                resource_runner.stop();
                                app.resources.disconnect();
                    app.completion.disconnect();
                                app.attention.disconnect();
                                oauth_service = None;
                                native_notifications = None;
                                app.connection = ConnectionState::WrongEpoch;
                            }
                            Err(error) => app.notice = Some(Notice::Diagnostic(error)),
                        }
                    }
                    Some(Err(error)) if history_job == Some(error.id()) => {
                        history_job = None;
                        if let Some(history) = app.chat.history.as_mut() {
                            history.fail(error.to_string());
                        }
                    }
                    Some(Err(error)) if !error.is_cancelled() => app.notice = Some(Notice::Diagnostic(error.to_string())),
                    _ => {}
                }
                dirty = true;
            }
            count = async {
                match notifications.as_mut() {
                    Some(receiver) => receiver.recv_many(&mut notification_batch, 32).await,
                    None => std::future::pending().await,
                }
            } => {
                // Apply each ordered notification before rebuilding and painting.
                // The bounded batch leaves other ready input and work admissible.
                if count == 0 { notifications = None; }
                for notice in notification_batch.drain(..) {
                match notice {
                    Notification::Catalog(notice) => {
                        if notice.kind == "project.catalog.changed" { app.project_catalog_changed(); app.bundle_catalog_changed(); }
                        if notice.kind == "plugin.terminal.changed" {
                            app.completion_catalog_changed();
                            app.apps.reload();
                        }
                        if notice.kind == "plugin.platform.changed" {
                            app.completion_catalog_changed();
                            app.plugins.changed();
                        }
                        if notice.kind == "model.provider.catalog.changed" {
                            app.providers.refresh();
                        }
                        if matches!(notice.kind.as_str(), "connection.catalog.changed" | "configuration.changed") {
                            app.models_catalog_changed();
                            app.connections.refresh();
                            app.chat.context.refresh();
                        }
                        if notice.kind == "session.catalog.changed" {
                            if let Some(id) = &notice.session_id {
                                app.sessions.invalidate(id);
                                app.apps_session_changed(id);
                            } else {
                                app.sessions.invalidate_all();
                            }
                            app.inbox.refresh();
                        }
                        app.notice = Some(Notice::Catalog { kind: notice.kind, revision: notice.revision.to_string() });
                    }
                    Notification::Observation(frame) => {
                        if app.chat.subscription.as_deref() == Some(frame.envelope().subscription_id)
                            && let maka_protocol::subscription::ObservationFrame::Resource(resource) = frame.as_ref() {
                            app.resources.observe(resource);
                        }
                        if let Err(error) = app.chat.accept(*frame) {
                            app.chat.error = Some(error.to_string());
                            if let Some(client) = &client { client.disconnect(); }
                        }
                        if app.chat.removed && let Some(id) = app.chat.session.clone() {
                            app.session_removed(&id, false);
                        }
                    }
                }
                }
                dirty = true;
            }
            error = async {
                match client.as_ref() {
                    Some(client) => client.closed().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(state) = &mut state { state.cancel_requests(); }
                jobs = JoinSet::new();
                history_job = None;
                app.abandon_interaction();
                app.abandon_pending_submissions();
                app.attachments.disconnect();
                app.apps.disconnect();
                watches.stop();
                    transcript_runner.stop();
                    resource_runner.stop();
                    app.resources.disconnect();
                    app.completion.disconnect();
                    app.attention.disconnect();
                app.bundle.disconnect();
                app.session_controls.disconnect();
                    app.plugins.disconnect();
                app.resume.disconnect();
                app.abandon_management();
                app.branch.disconnect();
                    app.revision.disconnect();
                app.management.oauth.abandon();
                app.abandon_onboarding();
                app.creating = false;
                client = None;
                notifications = None;
                oauth_service = None;
                native_notifications = None;
                app.status = None;
                app.refreshing = false;
                app.connection = ConnectionState::Failed(error.to_string());
                app.chat.disconnect(error.to_string());
                dirty = true;
            }
            delivery = transcript_deliveries.recv() => {
                checkpoint_impact = state::Impact::Reading;
                if let Some(delivery) = delivery {
                    dirty |= app.apps_transcript_delivery(delivery);
                }
            }
            change = changes.recv() => {
                match change {
                    Some(apps::io::Change::Stale(watch)) => app.apps_changed(&watch),
                    Some(apps::io::Change::Ended(watch)) => watches.ended(&watch),
                    Some(apps::io::Change::Failed(watch)) => app.apps_observation_failed(watch.owner),
                    None => {}
                }
                dirty = true;
            }
            _ = termination_signal(
                #[cfg(unix)]
                &mut terminate
            ) => break,
        }
        app.checkpoint_changed(checkpoint_impact);
    }
    app.attachments.disconnect();
    app.apps.disconnect();
    watches.stop();
    app.resources.disconnect();
    app.completion.disconnect();
    app.attention.disconnect();
    let resources_settled = resource_runner.shutdown().await;
    let _ = transcript_runner.shutdown().await;
    let _ = watches.shutdown().await;
    app.bundle.disconnect();
    app.session_controls.disconnect();
    app.plugins.disconnect();
    app.resume.disconnect();
    while completion_jobs.join_next().await.is_some() {}
    attachment_jobs.abort_all();
    jobs.abort_all();
    // Once Save was pressed, finish the bounded local write and checkpoint the
    // resulting choice before exit; dropping Host jobs must not strand it.
    while let Some(completed) = theme_jobs.join_next().await {
        let (request, result) = completed?;
        app.theme.complete(request, result);
        flushed = false;
    }
    if let Some(client) = client {
        client.disconnect();
    }
    if !flushed && let Some(state) = &mut state {
        state.finish(&mut app).await?;
    }
    if !resources_settled {
        return Err(app.i18n.text("resources-cleanup-unknown").into());
    }
    Ok(())
}

fn close_observation(jobs: &mut JoinSet<Completed>, client: Client, id: String) {
    jobs.spawn(async move {
        if client.close_subscription(&id).await.is_err() {
            client.disconnect();
        }
        Completed::ObservationClosed
    });
}

async fn termination_signal(#[cfg(unix)] signal: &mut tokio::signal::unix::Signal) {
    #[cfg(unix)]
    {
        signal.recv().await;
    }
    #[cfg(not(unix))]
    {
        std::future::pending::<()>().await;
    }
}
