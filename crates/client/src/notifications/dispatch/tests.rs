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

fn call(id: &str, destination: Value) -> Value {
    json!({"kind":"client.capability.service_call","registrationId":"registration", "invocationId":id,
        "serviceId":SERVICE_ID,"version":SERVICE_VERSION,"method":"send",
        "input":{"packageId":"maka.scheduler","notification":{"id":"reminder","title":"Reminder","body":"Private reminder body","destination":destination}}})
}
fn prepared() -> (Dispatch, NativeNotificationService) {
    let (sender, service) = NativeNotificationService::channel("registration".into());
    let mut dispatch = Dispatch::default();
    dispatch.prepare("registration", sender).unwrap();
    (dispatch, service)
}

#[tokio::test]
async fn notifications_need_host_admission_and_visible_consumer_acknowledgement() {
    let (mut dispatch, mut service) = prepared();
    let accepted = dispatch
        .frame(&call("one", json!({"kind":"local"})))
        .unwrap()
        .unwrap();
    assert_eq!(accepted["kind"], "client.capability.accepted");
    assert!(service.requests.try_recv().is_err());
    dispatch
        .frame(&json!({"kind":"client.capability.admitted","invocationId":"one"}))
        .unwrap();
    let mut delivered = service.recv().await.unwrap();
    assert_eq!(delivered.notice.package_id, "maka.scheduler");
    assert_eq!(delivered.notice.notification.body, "Private reminder body");
    let mut completion = Box::pin(dispatch.completion());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(completion.as_mut().poll(cx).is_pending())).await
    );
    assert!(delivered.acknowledge_presented());
    let result = completion.await;
    assert_eq!(result["invocationId"], "one");
    assert_eq!(result["result"]["structuredContent"], json!({"ok":true}));
    assert!(!result.to_string().contains("Private reminder"));
    assert!(dispatch.current_control(&result));
    dispatch
        .frame(&json!({"kind":"client.capability.release","invocationId":"one"}))
        .unwrap();
    assert!(!dispatch.current_control(&result));
}

#[tokio::test]
async fn channels_bad_payloads_and_cancelled_deliveries_never_acknowledge_success() {
    let (mut dispatch, mut service) = prepared();
    for (id, destination) in [
        (
            "channel",
            json!({"kind":"channel","channel":"mail","recipient":"someone"}),
        ),
        ("bad", json!({"kind":"local","extra":true})),
    ] {
        let rejected = dispatch.frame(&call(id, destination)).unwrap().unwrap();
        assert_eq!(rejected["kind"], "client.capability.rejected");
        assert!(service.requests.try_recv().is_err());
        dispatch
            .frame(&json!({"kind":"client.capability.release","invocationId":id}))
            .unwrap();
    }
    dispatch
        .frame(&call("cancelled", json!({"kind":"local"})))
        .unwrap();
    dispatch
        .frame(&json!({"kind":"client.capability.admitted","invocationId":"cancelled"}))
        .unwrap();
    let mut delivered = service.recv().await.unwrap();
    dispatch
        .frame(&json!({"kind":"client.capability.cancel","invocationId":"cancelled"}))
        .unwrap();
    assert!(delivered.is_cancelled());
    assert!(!delivered.acknowledge_presented());
    assert!(!dispatch.current_control(&json!({"invocationId":"cancelled"})));
}

#[tokio::test]
async fn concurrent_notifications_settle_independently_and_failed_consumers_do_not_fake_delivery() {
    let (mut dispatch, mut service) = prepared();
    for id in ["first", "second"] {
        dispatch.frame(&call(id, json!({"kind":"local"}))).unwrap();
        dispatch
            .frame(&json!({"kind":"client.capability.admitted","invocationId":id}))
            .unwrap();
    }
    let first = service.recv().await.unwrap();
    let mut second = service.recv().await.unwrap();
    drop(first);
    let failed = dispatch.completion().await;
    assert_eq!(failed["invocationId"], "first");
    assert_eq!(failed["kind"], "client.capability.failed");
    assert!(second.acknowledge_presented());
    let completed = dispatch.completion().await;
    assert_eq!(completed["invocationId"], "second");
    assert_eq!(completed["kind"], "client.capability.result");
}
