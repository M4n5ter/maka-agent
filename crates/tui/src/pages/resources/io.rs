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
use maka_client::{Client, ClientError, RequestFailure};
pub enum Output {
    Query(Box<ResourceQueryResult>),
    Started(Box<ShellSnapshot>),
    Stopped,
}
pub struct Failure {
    pub unknown: bool,
}
pub async fn execute(client: &Client, request: &Request) -> Result<Output, Failure> {
    if client.identity.root_id != request.target.root
        || client.identity.host_epoch != request.target.epoch
    {
        return Err(Failure { unknown: false });
    }
    let result = match &request.work {
        Work::Query(input) | Work::Check(input) => client
            .resource_query(input.clone())
            .await
            .map(|output| Output::Query(Box::new(output))),
        Work::Start(input) => client
            .resource_start(input.clone())
            .await
            .map(|result| Output::Started(Box::new(result.resource))),
        Work::Stop(input) => client
            .resource_stop(input.clone())
            .await
            .map(|()| Output::Stopped),
    };
    result.map_err(|error| {
        let uncertain = matches!(&error, RequestFailure::Unknown(_))
            || matches!(&error, RequestFailure::Rejected(ClientError::Rejected(error))
                if matches!(error.code, maka_protocol::OperationErrorCode::InternalFailure
                    | maka_protocol::OperationErrorCode::OperationUnavailable
                    | maka_protocol::OperationErrorCode::OutcomeUnknown
                    | maka_protocol::OperationErrorCode::CommitOutcomeUnknown));
        Failure {
            unknown: request.needs_checkpoint() && uncertain,
        }
    })
}
