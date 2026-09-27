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

use super::super::connection::pair_with;
use maka_client::{Client, ClientError, NativeServices, Notification, RequestFailure};
use maka_protocol::Operation;
use maka_transport::ndjson::{NdjsonReader, NdjsonWriter};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

struct Fixture {
    client: Client,
    notices: tokio::sync::mpsc::Receiver<Notification>,
    services: NativeServices,
    reader: NdjsonReader<ReadHalf<DuplexStream>>,
    writer: NdjsonWriter<WriteHalf<DuplexStream>>,
}
impl Fixture {
    async fn new() -> Self {
        let (client, notices, mut reader, mut writer) = pair_with(maka_client::Operations).await;
        let publishing = tokio::spawn({
            let client = client.clone();
            async move { client.publish_native_services().await }
        });
        let frame = reader.read().await.unwrap().unwrap();
        assert_eq!(frame["operation"], "client.capability.replace");
        assert_eq!(frame["input"]["offers"], json!([]));
        assert_eq!(
            frame["input"]["services"],
            json!([{"serviceId":"oauth_presentation","version":"1"},{"serviceId":"maka_notifications","version":"1"}])
        );
        writer.write(&json!({"requestId":frame["requestId"],"operation":frame["operation"],"ok":true,"result":{"registrationId":frame["input"]["registrationId"],"revision":1}})).await.unwrap();
        let services = publishing.await.unwrap().unwrap();
        assert_eq!(
            services.oauth.registration_id,
            services.notifications.registration_id
        );
        Self {
            client,
            notices,
            services,
            reader,
            writer,
        }
    }
    fn oauth(&self, id: &str) -> Value {
        json!({"kind":"client.capability.service_call","registrationId":self.services.oauth.registration_id,"invocationId":id,"serviceId":"oauth_presentation","version":"1","method":"open_external","input":{"url":"https://login.example/device","stateHint":"PRIVATE-CODE"}})
    }
    fn notification(&self, id: &str) -> Value {
        json!({"kind":"client.capability.service_call","registrationId":self.services.notifications.registration_id,"invocationId":id,"serviceId":"maka_notifications","version":"1","method":"send","input":{"packageId":"maka.scheduler","notification":{"id":id,"title":"Reminder","body":"Private notification body","destination":{"kind":"local"}}}})
    }
    async fn read(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(1), self.reader.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }
    async fn send_control(&mut self, kind: &str, id: &str) {
        self.writer
            .write(&json!({"kind":format!("client.capability.{kind}"),"invocationId":id}))
            .await
            .unwrap();
    }
    async fn barrier(&mut self) {
        let wake = tokio::spawn({
            let client = self.client.clone();
            async move { client.request(Operation::HostWake, json!({})).await }
        });
        let frame = self.read().await;
        assert_eq!(frame["operation"], "host.wake");
        self.writer.write(&json!({"requestId":frame["requestId"],"operation":frame["operation"],"ok":true,"result":{}})).await.unwrap();
        wake.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn native197_one_publication_routes_concurrent_services_and_fences_late_acknowledgements() {
    let mut f = Fixture::new().await;
    f.writer.write(&f.oauth("oauth")).await.unwrap();
    f.writer.write(&f.notification("notice")).await.unwrap();
    let mut accepted = BTreeSet::new();
    for _ in 0..2 {
        let frame = f.read().await;
        assert_eq!(frame["kind"], "client.capability.accepted");
        accepted.insert(frame["invocationId"].as_str().unwrap().to_owned());
    }
    assert_eq!(accepted, BTreeSet::from(["notice".into(), "oauth".into()]));
    tokio::select! {biased; _ = f.services.oauth.recv() => panic!("OAuth shown before admission"), _ = f.services.notifications.recv() => panic!("notification shown before admission"), _ = std::future::ready(()) => {}}
    f.send_control("admitted", "oauth").await;
    f.send_control("admitted", "notice").await;
    let oauth = tokio::time::timeout(Duration::from_secs(1), f.services.oauth.recv())
        .await
        .unwrap()
        .unwrap();
    let mut notice = tokio::time::timeout(Duration::from_secs(1), f.services.notifications.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notice.notice.notification.body, "Private notification body");
    f.barrier().await;
    f.writer
        .write(&json!({"kind":"configuration.changed","revision":7}))
        .await
        .unwrap();
    assert!(matches!(f.notices.recv().await, Some(Notification::Catalog(n)) if n.revision == 7));
    assert!(notice.acknowledge_presented());
    let completed = f.read().await;
    assert_eq!(completed["invocationId"], "notice");
    assert_eq!(completed["result"]["structuredContent"], json!({"ok":true}));
    assert!(!completed.to_string().contains("Private notification"));
    f.send_control("cancel", "oauth").await;
    f.barrier().await;
    assert!(oauth.is_cancelled());
    assert!(!oauth.acknowledge_presented());
    f.send_control("release", "notice").await;
    f.send_control("release", "oauth").await;
    f.barrier().await;
    assert!(notice.is_cancelled());
    assert!(!notice.acknowledge_presented());
    f.writer.write(&json!({"kind":"client.capability.registration_release","registrationId":f.services.oauth.registration_id})).await.unwrap();
    assert!(f.services.oauth.recv().await.is_none());
    assert!(f.services.notifications.recv().await.is_none());
    f.barrier().await;
    f.client.disconnect();
}

#[tokio::test]
async fn native197_rejecting_channel_or_second_publication_preserves_oauth_and_local_delivery() {
    let mut f = Fixture::new().await;
    assert!(matches!(
        f.client.publish_oauth_presentation().await,
        Err(RequestFailure::Unknown(ClientError::Protocol(_)))
    ));
    // The refused publisher never sends a second replace or owns either receiver.
    f.barrier().await;
    let mut channel = f.notification("channel");
    channel["input"]["notification"]["destination"] =
        json!({"kind":"channel","channel":"mail","recipient":"somewhere"});
    f.writer.write(&channel).await.unwrap();
    assert_eq!(f.read().await["kind"], "client.capability.rejected");
    f.send_control("release", "channel").await;
    f.writer.write(&f.oauth("login")).await.unwrap();
    assert_eq!(f.read().await["invocationId"], "login");
    f.send_control("admitted", "login").await;
    assert!(
        f.services
            .oauth
            .recv()
            .await
            .unwrap()
            .acknowledge_presented()
    );
    assert_eq!(
        f.read().await["result"]["structuredContent"],
        json!({"kind":"presented"})
    );
    f.send_control("release", "login").await;
    f.writer.write(&f.notification("local")).await.unwrap();
    assert_eq!(f.read().await["invocationId"], "local");
    f.send_control("admitted", "local").await;
    let mut delivery = f.services.notifications.recv().await.unwrap();
    assert!(delivery.acknowledge_presented());
    assert_eq!(
        f.read().await["result"]["structuredContent"],
        json!({"ok":true})
    );
    f.client.disconnect();
}

#[tokio::test]
async fn native197_duplicate_invocation_across_services_and_dropped_consumers_close_the_connection()
{
    let mut f = Fixture::new().await;
    f.writer.write(&f.oauth("collision")).await.unwrap();
    f.read().await;
    f.send_control("admitted", "collision").await;
    let oauth = f.services.oauth.recv().await.unwrap();
    f.writer.write(&f.notification("collision")).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), f.client.closed())
            .await
            .unwrap(),
        ClientError::Protocol(_)
    ));
    assert!(oauth.is_cancelled());
    assert!(!oauth.acknowledge_presented());

    for drop_oauth in [false, true] {
        let f = Fixture::new().await;
        let NativeServices {
            oauth,
            notifications,
        } = f.services;
        if drop_oauth {
            drop(oauth);
            tokio::time::timeout(Duration::from_secs(1), f.client.closed())
                .await
                .unwrap();
            drop(notifications);
        } else {
            drop(notifications);
            tokio::time::timeout(Duration::from_secs(1), f.client.closed())
                .await
                .unwrap();
            drop(oauth);
        }
    }
}
