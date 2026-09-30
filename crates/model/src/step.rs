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

pub use maka_runtime::model::assembly::StepBuilder;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Normalizer;
    use maka_runtime::model::TextKind;
    use maka_runtime::model::{ModelEvent, ModelFinishReason, ModelPart, ModelUsage};
    use serde_json::json;

    #[test]
    fn interleaved_parts_preserve_metadata_and_require_complete_streams() {
        let events = [
            json!({"type":"reasoning-start","id":"r","providerMetadata":{"anthropic":{"signature":"initial","other":1}}}),
            json!({"type":"text-start","id":"t"}),
            json!({"type":"reasoning-delta","id":"r","delta":"think"}),
            json!({"type":"text-delta","id":"t","delta":"answer"}),
            json!({"type":"reasoning-end","id":"r","providerMetadata":{"anthropic":{"signature":"signed"}}}),
            json!({"type":"text-end","id":"t","providerMetadata":{"openai":{"itemId":"item"}}}),
            json!({"type":"finish","finishReason":{"unified":"stop"},"usage":{"inputTokens":{"total":3},"outputTokens":{"total":4,"reasoning":2}}}),
        ];
        for length in [4, 6, 7] {
            let mut normalizer = Normalizer::default();
            let mut builder = StepBuilder::default();
            for event in &events[..length] {
                if let Some(event) = normalizer.push(event.clone()).unwrap() {
                    builder.push(event).unwrap();
                }
            }
            assert_eq!(normalizer.end().is_ok(), length == 7);
            let result = builder.finish();
            assert_eq!(result.is_ok(), length == 7);
            if let Ok(step) = result {
                assert_eq!(
                    step.parts,
                    vec![
                        ModelPart::Text {
                            text_kind: TextKind::Thinking,
                            text: "think".into(),
                            provider_options: Some(
                                json!({"anthropic":{"signature":"signed","other":1}})
                            )
                        },
                        ModelPart::Text {
                            text_kind: TextKind::Text,
                            text: "answer".into(),
                            provider_options: Some(json!({"openai":{"itemId":"item"}}))
                        },
                    ]
                );
                assert_eq!(step.usage.reasoning_tokens, Some(2));
            }
        }
        for already_finished in [false, true] {
            let mut builder = StepBuilder::default();
            let finish = ModelEvent::Finished {
                reason: ModelFinishReason::Stop,
                usage: ModelUsage::default(),
                provider_options: None,
            };
            if already_finished {
                builder.push(finish.clone()).unwrap();
            }
            assert!(
                builder
                    .push(ModelEvent::PartFinished {
                        id: "unknown".into(),
                        provider_options: None,
                    })
                    .is_err()
            );
            assert!(builder.push(finish).is_err());
            assert!(builder.finish().is_err());
        }
        for reason in ["content-filter", "error", "unknown"] {
            let event = json!({"type":"finish","finishReason":{"unified":reason},"usage":{}});
            assert!(Normalizer::default().push(event).is_err());
            assert!(serde_json::from_value::<ModelFinishReason>(json!(reason)).is_err());
        }
        for (reason, wire) in [
            (ModelFinishReason::Stop, "stop"),
            (ModelFinishReason::ToolCalls, "tool-calls"),
            (ModelFinishReason::Length, "length"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<ModelFinishReason>(json!(wire)).unwrap(),
                reason
            );
            let finish = Normalizer::default()
                .push(json!({
                    "type":"finish", "finishReason":{"unified":wire},
                    "usage":{"inputTokens":{"total":3},"outputTokens":{"total":8}}
                }))
                .unwrap()
                .unwrap();
            let mut builder = StepBuilder::default();
            builder.push(finish).unwrap();
            let step = builder.finish().unwrap();
            assert_eq!(step.finish_reason, reason);
            assert_eq!(step.usage.output_tokens, Some(8));
        }
    }
}
