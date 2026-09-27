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

use super::*;
use crate::ClientError;
use maka_protocol::{
    Outcome,
    capability::{self, HostFrame},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::{Future, poll_fn},
    pin::Pin,
    sync::Arc,
    task::Poll,
};
use tokio::sync::Semaphore;

struct Invocation {
    cancelled: CancellationToken,
    stage: Stage,
}
enum Stage {
    Accepted {
        notice: Notice,
        retained: OwnedSemaphorePermit,
        slot: mpsc::OwnedPermit<NotificationDelivery>,
    },
    Presenting(oneshot::Receiver<()>),
    Finished,
}
#[derive(Default)]
pub(crate) struct Dispatch {
    registration: Option<(String, mpsc::Sender<NotificationDelivery>)>,
    invocations: BTreeMap<String, Invocation>,
    retained: Option<Arc<Semaphore>>,
}
impl Drop for Dispatch {
    fn drop(&mut self) {
        for invocation in self.invocations.values() {
            invocation.cancelled.cancel();
        }
    }
}
impl Dispatch {
    pub fn prepare(
        &mut self,
        id: &str,
        sender: mpsc::Sender<NotificationDelivery>,
    ) -> Result<(), ClientError> {
        if self.registration.is_some() {
            return Err(invalid("native notifications already registered"));
        }
        self.registration = Some((id.into(), sender));
        self.retained = Some(Arc::new(Semaphore::new(RETAINED_BYTES)));
        Ok(())
    }
    pub fn complete(&mut self, id: &str, outcome: &Outcome) -> Result<(), ClientError> {
        if self
            .registration
            .as_ref()
            .is_none_or(|(registered, _)| registered != id)
        {
            return Err(invalid("notification registration changed identity"));
        }
        match outcome {
            Outcome::Success { result } => {
                if capability::decode_registration_result(result)
                    .map_err(invalid)?
                    .registration_id
                    != id
                {
                    return Err(invalid(
                        "notification registration response changed identity",
                    ));
                }
            }
            Outcome::Failure { .. } => {
                if !self.invocations.is_empty() {
                    return Err(invalid(
                        "rejected notification registration already invoked",
                    ));
                }
                self.registration = None;
            }
        }
        Ok(())
    }
    pub fn consumer(&self) -> Option<mpsc::Sender<NotificationDelivery>> {
        self.registration.as_ref().map(|(_, sender)| sender.clone())
    }
    pub fn owns(&self, id: &str) -> bool {
        self.invocations.contains_key(id)
    }
    pub fn current_control(&self, frame: &Value) -> bool {
        frame["invocationId"]
            .as_str()
            .and_then(|id| self.invocations.get(id))
            .is_some_and(|invocation| !invocation.cancelled.is_cancelled())
    }
    pub async fn completion(&mut self) -> Value {
        poll_fn(|cx| {
            for (id, invocation) in &mut self.invocations {
                if let Stage::Presenting(receiver) = &mut invocation.stage {
                    match Pin::new(receiver).poll(cx) {
                        Poll::Ready(result) => {
                            invocation.stage = Stage::Finished;
                            return Poll::Ready(if result.is_ok() {
                                json!({"kind":"client.capability.result","invocationId":id,"result":{"content":[],"structuredContent":{"ok":true}}})
                            } else { failed(id) });
                        }
                        Poll::Pending => {}
                    }
                }
            }
            Poll::Pending
        }).await
    }
    pub fn frame(&mut self, value: &Value) -> Result<Option<Value>, ClientError> {
        match capability::decode_host_frame(value).map_err(invalid)? {
            HostFrame::ServiceCall {
                invocation_id,
                registration_id,
                service_id,
                version,
                method,
                input,
            } => {
                let (id, sender) = self
                    .registration
                    .as_ref()
                    .ok_or_else(|| invalid("unregistered native notification"))?;
                if *id != registration_id || self.invocations.contains_key(&invocation_id) {
                    return Err(invalid("notification registration or invocation mismatch"));
                }
                // This is a bound on live call bookkeeping, not on service or
                // plugin inventory. Rejected calls also await Host release.
                if self.invocations.len() >= maka_protocol::MAX_IN_FLIGHT_DOMAIN_REQUESTS {
                    return Err(invalid("too many unreleased notification invocations"));
                }
                let notice =
                    (service_id == SERVICE_ID && version == SERVICE_VERSION && method == "send")
                        .then(|| Notice::decode(Value::Object(input)).ok())
                        .flatten();
                let accepted = notice.and_then(|notice| {
                    let retained = self
                        .retained
                        .as_ref()?
                        .clone()
                        .try_acquire_many_owned(notice.bytes() as u32)
                        .ok()?;
                    let slot = sender.clone().try_reserve_owned().ok()?;
                    Some(Stage::Accepted {
                        notice,
                        retained,
                        slot,
                    })
                });
                let allowed = accepted.is_some();
                self.invocations.insert(
                    invocation_id.clone(),
                    Invocation {
                        cancelled: CancellationToken::new(),
                        stage: accepted.unwrap_or(Stage::Finished),
                    },
                );
                Ok(Some(if allowed {
                    json!({"kind":"client.capability.accepted","invocationId":invocation_id,"admissionEvidence":{"kind":"none"}})
                } else {
                    json!({"kind":"client.capability.rejected","invocationId":invocation_id,"message":"Local notification is unavailable"})
                }))
            }
            HostFrame::Admitted { invocation_id } => {
                let invocation = self
                    .invocations
                    .get_mut(&invocation_id)
                    .ok_or_else(|| invalid("unknown notification admission"))?;
                let Stage::Accepted {
                    notice,
                    retained,
                    slot,
                } = std::mem::replace(&mut invocation.stage, Stage::Finished)
                else {
                    return Err(invalid("notification admitted out of order"));
                };
                let (presented, receiver) = oneshot::channel();
                slot.send(NotificationDelivery {
                    notice,
                    cancelled: invocation.cancelled.clone(),
                    presented: Some(presented),
                    _retained: retained,
                });
                invocation.stage = Stage::Presenting(receiver);
                Ok(None)
            }
            HostFrame::Cancel { invocation_id } => {
                let invocation = self
                    .invocations
                    .get_mut(&invocation_id)
                    .ok_or_else(|| invalid("unknown notification cancellation"))?;
                invocation.cancelled.cancel();
                invocation.stage = Stage::Finished;
                Ok(None)
            }
            HostFrame::Release { invocation_id } => {
                self.invocations
                    .remove(&invocation_id)
                    .ok_or_else(|| invalid("unknown notification release"))?
                    .cancelled
                    .cancel();
                Ok(None)
            }
            HostFrame::RegistrationRelease { registration_id } => {
                if self
                    .registration
                    .as_ref()
                    .is_none_or(|(id, _)| *id != registration_id)
                {
                    return Err(invalid("unknown notification registration release"));
                }
                for (_, invocation) in std::mem::take(&mut self.invocations) {
                    invocation.cancelled.cancel();
                }
                self.registration = None;
                Ok(None)
            }
            _ => Err(invalid("unsupported native notification frame")),
        }
    }
}
fn invalid(error: impl std::fmt::Display) -> ClientError {
    ClientError::Protocol(error.to_string())
}
fn failed(id: &str) -> Value {
    json!({"kind":"client.capability.failed","invocationId":id,"message":"Local notification was not presented"})
}

#[cfg(test)]
mod tests;
