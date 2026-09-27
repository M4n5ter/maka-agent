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
use maka_plugins::session::{
    View as Session,
    catalog::{List, Queries},
};

/// A page carries a reference to canonical metadata, never an execution template.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(in crate::plugin::terminal) enum Selector {
    #[default]
    Local,
    Session {
        selection: Selection,
        id: String,
        revision: u64,
    },
}
impl Selector {
    pub(in crate::plugin::terminal) fn capture(
        selection: Selection,
        session: &Session,
    ) -> Result<Self, Error> {
        from_session(selection, session)?;
        Ok(Self::Session {
            selection,
            id: session.session_id.clone(),
            revision: session.revision,
        })
    }
    fn current(&self, session: &Session) -> Option<Effect> {
        match self {
            Self::Session {
                selection,
                id,
                revision,
            } if *id == session.session_id && *revision == session.revision => {
                from_session(*selection, session).ok()
            }
            _ => None,
        }
    }
    pub(in crate::plugin::terminal) async fn resolve(
        &self,
        sessions: &dyn Queries,
        caller: &Caller,
        locale: &str,
    ) -> Result<Option<Effect>, Error> {
        let Self::Session { id, .. } = self else {
            return Ok(Some(Effect::Notify(Notification::Local)));
        };
        let owned = caller
            .views
            .authorize(Consent {
                operation_id: uuid::Uuid::new_v4(),
                title: Text::localized(
                    "Inspect scheduled task target",
                    "检查计划任务目标",
                    "檢查排程任務目標",
                )
                .resolve(locale)
                .into(),
                target: Target::Profile,
                capabilities: [Capability::ReadSessions].into(),
            })
            .await?;
        let result = async {
            let mut query = List::default();
            loop {
                let page = match sessions.list(owned.scope(), query).await {
                    Ok(page) => page,
                    Err(maka_plugins::execution::CommandError::Conflict) => return Ok(None),
                    Err(error) => return Err(Error::Provider(error.to_string())),
                };
                if let Some(found) = page
                    .entries
                    .iter()
                    .find(|entry| &entry.session.session_id == id)
                {
                    return Ok(self.current(&found.session));
                }
                let Some(cursor) = page.next_cursor else {
                    return Ok(None);
                };
                query = List {
                    revision: Some(page.revision),
                    cursor: Some(cursor),
                    include_archived: false,
                };
            }
        }
        .await;
        owned
            .finish()
            .await
            .map_err(|_| Error::CleanupUnconfirmed)?;
        result
    }
}

#[cfg(test)]
mod tests;
