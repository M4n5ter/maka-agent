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

use maka_protocol::interaction::*;
use serde_json::{Value, json};

fn request() -> Value {
    json!({"kind":"client_capability","toolUseId":"tool",
        "target":{"providerId":"provider","contractId":"contract","serverId":"server",
        "toolName":"browser","capability":"browser","scope":{"kind":"browser_origin","origin":"https://example.com"}}})
}
fn pending() -> Value {
    json!({"schemaVersion":1,"sessionId":"session","turnId":"turn","runId":"run",
        "interactionId":"interaction","request":request(),"revision":1,"status":"pending","outcome":null})
}

#[test]
fn canonical_record_validation_and_projection_derive_resolution() {
    let mut record = InteractionRecord {
        session_id: "session".into(),
        turn_id: "turn".into(),
        run_id: "run".into(),
        request_id: "interaction".into(),
        created_at: 1,
        request: decode_request(&request()).unwrap(),
        outcome: None,
    };
    assert_eq!(
        serde_json::to_value(InteractionSnapshot::from_record(&record).unwrap()).unwrap(),
        pending()
    );
    record.outcome = Some(InteractionOutcome::Closure {
        reason: ClosureReason::HostRestarted,
        committed_at: 2,
    });
    let wire = serde_json::to_value(InteractionSnapshot::from_record(&record).unwrap()).unwrap();
    assert_eq!(wire["revision"], 2);
    assert_eq!(wire["status"], "closed");
    assert_eq!(
        decode_snapshot(&wire).unwrap(),
        InteractionSnapshot::from_record(&record).unwrap()
    );
    assert!(decode_answered_snapshot(&wire).is_err());
    record.created_at = MAX_SAFE_INTEGER + 1;
    assert!(InteractionSnapshot::from_record(&record).is_err());
    // Malformed questions and unsupported producers remain invalid.
    assert!(decode_request(&json!({"kind":"question","questions":[]})).is_err());
    assert!(decode_answer(&json!({"kind":"permission","decision":"allow"})).is_err());
}
