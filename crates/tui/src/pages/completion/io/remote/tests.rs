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
use crate::{
    editor::{Editor, completion::Kind},
    pages::completion::{
        DraftKey,
        io::{Cancellation, Job},
        model::Context,
    },
};
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

type Reader = BufReader<ReadHalf<DuplexStream>>;
type Writer = WriteHalf<DuplexStream>;
const ROOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
async fn read(reader: &mut Reader) -> Value {
    let mut line = String::new();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap()
            > 0
    );
    serde_json::from_str(&line).unwrap()
}
async fn write(writer: &mut Writer, value: Value) {
    writer
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
}
async fn reply(writer: &mut Writer, request: &Value, result: Value) {
    write(writer, json!({"requestId":request["requestId"],"operation":request["operation"],"ok":true,"result":result})).await;
}
async fn connect() -> (
    Client,
    Reader,
    Writer,
    tokio::sync::mpsc::Receiver<maka_client::Notification>,
) {
    let (local, remote) = tokio::io::duplex(64 * 1024);
    let (reader, mut writer) = tokio::io::split(remote);
    let mut reader = BufReader::new(reader);
    let connecting = tokio::spawn(Client::connect(
        local,
        ROOT,
        "epoch",
        maka_client::Operations,
    ));
    read(&mut reader).await;
    write(&mut writer, json!({"kind":"accepted","rootId":ROOT,"hostEpoch":"epoch","connectionId":"test",
        "selectedProtocol":maka_protocol::PROTOCOL_VERSION,"compatibilityEpoch":maka_protocol::COMPATIBILITY_EPOCH,
        "compositionId":maka_protocol::COMPOSITION_ID,"compositionRevision":"test","state":"ready"})).await;
    let (client, notifications) = connecting.await.unwrap().unwrap();
    (client, reader, writer, notifications)
}
fn request() -> Request {
    Request {
        id: 1,
        generation: 1,
        context: Context {
            root: ROOT.into(),
            epoch: "epoch".into(),
            draft: DraftKey {
                session: "chat".into(),
                input: None,
                display: false,
            },
            session: "chat".into(),
            token: Editor::default().insertion_token(Kind::Reference),
        },
        locale: "en".into(),
        job: Job::Skills { page: None },
        cancel: Cancellation::default(),
    }
}
fn run(client: Client, request: Request) -> tokio::task::JoinHandle<Result<Value, String>> {
    tokio::spawn(async move {
        remote(
            &client,
            &request,
            RemoteBinding::Package {
                package_id: "records".into(),
                method: "resources".into(),
                session_id: Some("chat".into()),
            },
            maka_plugins::remote::Target {
                entry_id: "records".into(),
                activation: "one".into(),
                registration: uuid::Uuid::new_v4(),
            },
            json!({"kind":"query"}),
        )
        .await
    })
}
async fn retired(client: &Client, reader: &mut Reader) {
    tokio::time::timeout(Duration::from_secs(1), client.closed())
        .await
        .unwrap();
    let mut line = String::new();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test(start_paused = true)]
async fn lost_open_retires_the_connection_before_a_late_document_can_be_orphaned() {
    let (client, mut reader, mut writer, _notifications) = connect().await;
    let job = run(client.clone(), request());
    let opening = read(&mut reader).await;
    assert_eq!(opening["input"]["kind"], "open_document");
    tokio::time::advance(Duration::from_secs(16)).await;
    assert!(
        job.await
            .unwrap()
            .unwrap_err()
            .contains("ownership is unknown")
    );
    retired(&client, &mut reader).await;
    let late = json!({"requestId":opening["requestId"],"operation":opening["operation"],"ok":true,
        "result":{"kind":"document","document":uuid::Uuid::new_v4()}});
    assert!(
        writer
            .write_all(format!("{late}\n").as_bytes())
            .await
            .is_err(),
        "late allocation cannot remain on a live connection"
    );
}

#[tokio::test(start_paused = true)]
async fn unconfirmed_close_retires_the_connection_and_takes_precedence_over_cancellation() {
    for rejected in [true, false] {
        let (client, mut reader, mut writer, _notifications) = connect().await;
        let request = request();
        let job = run(client.clone(), request.clone());
        let opening = read(&mut reader).await;
        request.cancel();
        let document = uuid::Uuid::new_v4();
        reply(
            &mut writer,
            &opening,
            json!({"kind":"document","document":document}),
        )
        .await;
        let closing = read(&mut reader).await;
        assert_eq!(closing["input"]["kind"], "close_document");
        assert_eq!(closing["input"]["document"], document.to_string());
        if rejected {
            write(
                &mut writer,
                json!({"requestId":closing["requestId"],"operation":closing["operation"],"ok":false,
                "error":{"code":"operation_conflict","message":"close not confirmed"}}),
            )
            .await;
        } else {
            tokio::time::advance(Duration::from_secs(16)).await;
        }
        assert!(
            job.await
                .unwrap()
                .unwrap_err()
                .contains("cleanup was not confirmed")
        );
        retired(&client, &mut reader).await;
    }
}

#[tokio::test(start_paused = true)]
async fn cancelling_during_open_still_closes_the_known_document_without_retiring_the_connection() {
    let (client, mut reader, mut writer, _notifications) = connect().await;
    let request = request();
    let job = run(client.clone(), request.clone());
    let opening = read(&mut reader).await;
    request.cancel();
    reply(
        &mut writer,
        &opening,
        json!({"kind":"document","document":uuid::Uuid::new_v4()}),
    )
    .await;
    let closing = read(&mut reader).await;
    assert_eq!(closing["input"]["kind"], "close_document");
    reply(&mut writer, &closing, json!({"kind":"closed"})).await;
    assert_eq!(job.await.unwrap().unwrap_err(), "completion-cancelled");
    assert!(
        tokio::time::timeout(Duration::from_millis(1), client.closed())
            .await
            .is_err()
    );
    client.disconnect();
}
