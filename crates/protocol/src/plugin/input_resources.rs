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

//! Typed values carried by the existing exact-target Remote document transport.
use crate::{ProtocolError, Result};
pub use maka_plugins::input::resources::{
    Reply as InputResourceReply, Request as InputResourceRequest,
};
use serde_json::Value;

pub fn decode_input_resource_reply(value: &Value) -> Result<InputResourceReply> {
    use maka_plugins::input::resources::{Page, Value as ResourceValue};
    let reply: InputResourceReply =
        serde_json::from_value(value.clone()).map_err(|e| ProtocolError::invalid(e.to_string()))?;
    match &reply {
        InputResourceReply::Page { items, next_cursor } => Page {
            items: items.clone(),
            next_cursor: next_cursor.clone(),
        }
        .validate(64)
        .map_err(|e| ProtocolError::invalid(e.to_string()))?,
        InputResourceReply::Resolved {
            source,
            selector,
            label,
            quote,
        } => {
            ResourceValue {
                selector: selector.clone(),
                label: label.clone(),
                quote: quote.clone(),
            }
            .validate()
            .map_err(|e| ProtocolError::invalid(e.to_string()))?;
            maka_runtime::input::validate_selection_sources(
                &maka_runtime::input::Selections::from([(
                    source.provider.clone(),
                    vec![selector.clone()],
                )]),
                std::slice::from_ref(source.as_ref()),
            )
            .map_err(ProtocolError::invalid)?;
        }
    }
    if serde_json::to_vec(value)
        .map_err(|e| ProtocolError::invalid(e.to_string()))?
        .len()
        > 64 * 1024
    {
        return Err(ProtocolError::invalid(
            "Input resource reply exceeds 64 KiB",
        ));
    }
    Ok(reply)
}
