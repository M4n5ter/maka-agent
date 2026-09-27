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

//! Native local notification delivery. Content is data, never an executable action.
mod dispatch;
pub(crate) use dispatch::Dispatch;
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub const SERVICE_ID: &str = "maka_notifications";
pub const SERVICE_VERSION: &str = "1";
const RETAINED_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalNotification {
    pub id: String,
    pub title: String,
    pub body: String,
    destination: Local,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Local {
    Local,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notice {
    pub package_id: String,
    pub notification: LocalNotification,
}
impl Notice {
    fn decode(input: serde_json::Value) -> Result<Self, ()> {
        if !input["notification"]["destination"]
            .as_object()
            .is_some_and(|value| {
                value.len() == 1 && value.get("kind").is_some_and(|kind| kind == "local")
            })
        {
            return Err(());
        }
        let notice: Self = serde_json::from_value(input).map_err(|_| ())?;
        let valid_id = |value: &str| {
            !value.is_empty() && value.len() <= 512 && value.chars().all(|ch| !ch.is_control())
        };
        if !valid_id(&notice.package_id)
            || !valid_id(&notice.notification.id)
            || notice.notification.title.trim().is_empty()
            || notice.notification.title.len() > 512
            || notice.notification.body.len() > 32 * 1024
        {
            return Err(());
        }
        Ok(notice)
    }
    fn bytes(&self) -> usize {
        self.package_id.len()
            + self.notification.id.len()
            + self.notification.title.len()
            + self.notification.body.len()
            + 512
    }
}

/// Keep this receipt with the inbox item until it is dismissed. Its permit bounds
/// retained content, including items whose delivery has already been acknowledged.
pub struct NotificationDelivery {
    pub notice: Notice,
    cancelled: CancellationToken,
    presented: Option<oneshot::Sender<()>>,
    _retained: OwnedSemaphorePermit,
}
impl NotificationDelivery {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }
    /// Only the consumer's committed visible frame may acknowledge delivery.
    pub fn acknowledge_presented(&mut self) -> bool {
        if self.cancelled.is_cancelled() {
            return false;
        }
        self.presented
            .take()
            .is_some_and(|presented| presented.send(()).is_ok())
    }
}

pub struct NativeNotificationService {
    pub registration_id: String,
    requests: mpsc::Receiver<NotificationDelivery>,
}
impl NativeNotificationService {
    pub(crate) fn channel(registration_id: String) -> (mpsc::Sender<NotificationDelivery>, Self) {
        let (sender, requests) = mpsc::channel(32);
        (
            sender,
            Self {
                registration_id,
                requests,
            },
        )
    }
    pub async fn recv(&mut self) -> Option<NotificationDelivery> {
        while let Some(request) = self.requests.recv().await {
            if !request.is_cancelled() {
                return Some(request);
            }
        }
        None
    }
}
