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

//! Display-only media summaries. Durable payloads and model history remain untouched.
use super::*;
use sha2::{Digest, Sha256};

pub(crate) async fn complete_page(
    client: &Client,
    subscription: &str,
    page: SessionTranscriptPage,
) -> Result<TranscriptBatch, Error> {
    let batch = client.complete_transcript_page(subscription, page).await?;
    // Summarizing and dropping multi-MiB inline media must never run on the UI thread.
    tokio::task::spawn_blocking(move || prepare(batch))
        .await
        .map_err(Into::into)
}

pub(super) fn prepare(mut batch: TranscriptBatch) -> TranscriptBatch {
    for row in &mut batch.rows {
        if row.value["type"] != "tool_result" || row.value["content"]["kind"] != "json" {
            continue;
        }
        let Some(parts) = row.value["content"]["value"]["content"].as_array() else {
            continue;
        };
        if !parts.iter().any(|part| {
            matches!(part["type"].as_str(), Some("image" | "audio"))
                && part["mimeType"].is_string()
                && part["data"].is_string()
                || part["type"] == "resource"
                    && part["resource"]["uri"].is_string()
                    && part["resource"]["blob"].is_string()
        }) {
            continue;
        }
        // Reuse the client's verified source digest instead of hashing every
        // multi-MiB image again. Fragments without a wire digest need one hash.
        let digest = row.payload_digest.clone().unwrap_or_else(|| {
            format!(
                "sha256:{:x}",
                Sha256::digest(serde_json::to_vec(&row.value).expect("JSON value"))
            )
        });
        let parts = row.value["content"]["value"]["content"]
            .as_array_mut()
            .unwrap();
        for part in parts {
            match part["type"].as_str() {
                Some("image" | "audio") if part["mimeType"].is_string() => {
                    if let Some(data) = part.get_mut("data") {
                        summarize(data, &digest);
                    }
                }
                Some("resource") if part["resource"]["uri"].is_string() => {
                    if let Some(blob) = part["resource"].get_mut("blob") {
                        summarize(blob, &digest);
                    }
                }
                _ => {}
            }
        }
    }
    batch
}

fn summarize(value: &mut Value, source_digest: &str) {
    let Some(data) = value.as_str() else {
        return;
    };
    // The digest preserves immutable-row identity, even for equal-length images.
    // Keep all text, MIME types and other structured evidence; only binary data
    // gets an explicit display marker instead of megabytes of encoded glyphs.
    *value = serde_json::json!({
        "display": "inline media omitted",
        "encodedBytes": data.len(),
        "sourceDigest": source_digest,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn summaries_preserve_metadata_text_and_binary_identity() {
        let row = json!({"type":"tool_result","content":{"kind":"json","value":{
            "content":[{"type":"text","text":"full text"},
                {"type":"image","mimeType":"image/png","data":"AAAA","custom":42},
                {"type":"audio","mimeType":"audio/wav","data":"BBBB"},
                {"type":"resource","resource":{"uri":"file:///sample","blob":"CCCC"}},
                {"type":"future","data":"untouched"}],
            "structuredContent":{"data":"not media"}}}});
        let project = |value| {
            prepare(TranscriptBatch {
                rows: vec![maka_client::transcript::TranscriptRow {
                    payload_digest: None,
                    sequence: 1,
                    value,
                }],
                next_cursor: None,
                through_sequence: Some(1),
            })
            .rows
            .remove(0)
            .value
        };
        let projected = project(row.clone());
        let parts = &projected["content"]["value"]["content"];
        assert_eq!(parts[0]["text"], "full text");
        assert_eq!(parts[1]["custom"], 42);
        assert_eq!(parts[1]["mimeType"], "image/png");
        assert_eq!(parts[1]["data"]["encodedBytes"], 4);
        assert!(parts[2]["data"]["sourceDigest"].is_string());
        assert!(parts[3]["resource"]["blob"]["sourceDigest"].is_string());
        assert_eq!(parts[4]["data"], "untouched");
        assert_eq!(
            projected["content"]["value"]["structuredContent"]["data"],
            "not media"
        );
        let mut changed = row;
        changed["content"]["value"]["content"][1]["data"] = json!("AAAB");
        assert_ne!(projected, project(changed));
        assert_eq!(
            projected,
            project(projected.clone()),
            "display preparation is idempotent"
        );
    }
}
