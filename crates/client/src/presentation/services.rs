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

//! One publication and transport lane for the native interface's services.
use super::{OAuthPresentation, Presentation, invalid};
use crate::{
    ClientError,
    notifications::{self, NotificationDelivery},
};
use maka_protocol::{
    Outcome,
    capability::{self, HostFrame},
    oauth,
};
use serde_json::Value;
use tokio::sync::mpsc;

pub(crate) struct Publication {
    pub oauth: mpsc::Sender<OAuthPresentation>,
    pub notifications: Option<mpsc::Sender<NotificationDelivery>>,
}
#[derive(Default)]
pub(crate) struct NativeDispatch {
    registration: Option<String>,
    oauth: Presentation,
    notifications: notifications::Dispatch,
    local_notifications: bool,
}
impl NativeDispatch {
    pub fn prepare(
        &mut self,
        input: &Value,
        publication: Publication,
    ) -> Result<String, ClientError> {
        if self.registration.is_some() {
            return Err(invalid("Native services already registered"));
        }
        let manifest = capability::decode_replace_input(input).map_err(invalid)?;
        let native = publication.notifications.is_some();
        let expected = if native { 2 } else { 1 };
        let services = manifest.services.as_deref().unwrap_or_default();
        if !manifest.offers.is_empty()
            || services.len() != expected
            || !services.iter().any(|service| {
                service.service_id == oauth::PRESENTATION_SERVICE_ID
                    && service.version == oauth::PRESENTATION_SERVICE_VERSION
            })
            || native
                && !services.iter().any(|service| {
                    service.service_id == notifications::SERVICE_ID
                        && service.version == notifications::SERVICE_VERSION
                })
        {
            return Err(invalid(
                "Native service publication does not match consumers",
            ));
        }
        let id = self.oauth.prepare(input, publication.oauth)?;
        if let Some(sender) = publication.notifications {
            self.notifications.prepare(&id, sender)?;
        }
        self.local_notifications = native;
        self.registration = Some(id.clone());
        Ok(id)
    }
    pub fn complete(&mut self, id: &str, outcome: &Outcome) -> Result<(), ClientError> {
        if self.registration.as_deref() != Some(id) {
            return Err(invalid("Native registration changed identity"));
        }
        self.oauth.complete(id, outcome)?;
        if self.local_notifications {
            self.notifications.complete(id, outcome)?;
        }
        if matches!(outcome, Outcome::Failure { .. }) {
            self.registration = None;
            self.local_notifications = false;
        }
        Ok(())
    }
    pub fn oauth_consumer(&self) -> Option<mpsc::Sender<OAuthPresentation>> {
        self.oauth.consumer()
    }
    pub fn notification_consumer(&self) -> Option<mpsc::Sender<NotificationDelivery>> {
        self.notifications.consumer()
    }
    pub fn current_control(&self, frame: &Value) -> bool {
        self.oauth.current_control(frame) || self.notifications.current_control(frame)
    }
    pub async fn completion(&mut self) -> Value {
        tokio::select! {
            frame = self.oauth.completion() => frame,
            frame = self.notifications.completion(), if self.local_notifications => frame,
        }
    }
    pub fn frame(&mut self, value: &Value) -> Result<Option<Value>, ClientError> {
        match capability::decode_host_frame(value).map_err(invalid)? {
            HostFrame::ServiceCall {
                invocation_id,
                service_id,
                ..
            } => {
                if self.oauth.owns(&invocation_id) || self.notifications.owns(&invocation_id) {
                    return Err(invalid("Native service invocation is already owned"));
                }
                if service_id == notifications::SERVICE_ID && self.local_notifications {
                    self.notifications.frame(value)
                } else {
                    // Preserve the OAuth-only publisher's explicit rejection of
                    // unsupported services, methods and versions.
                    self.oauth.frame(value)
                }
            }
            HostFrame::Admitted { invocation_id }
            | HostFrame::Cancel { invocation_id }
            | HostFrame::Release { invocation_id } => {
                if self.notifications.owns(&invocation_id) {
                    self.notifications.frame(value)
                } else if self.oauth.owns(&invocation_id) {
                    self.oauth.frame(value)
                } else {
                    Err(invalid("Native service invocation has no owner"))
                }
            }
            HostFrame::RegistrationRelease { registration_id } => {
                if self.registration.as_deref() != Some(&registration_id) {
                    return Err(invalid("Native registration release changed identity"));
                }
                self.oauth.frame(value)?;
                if self.local_notifications {
                    self.notifications.frame(value)?;
                }
                self.registration = None;
                self.local_notifications = false;
                Ok(None)
            }
            _ => Err(invalid("Unpublished client capability invoked")),
        }
    }
}
