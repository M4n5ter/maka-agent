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

use maka_computer_use::{
    Session,
    protocol::{Command, InventoryOptions},
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn closed_or_cancelled_session_cannot_start_native_work() {
    let driver = std::sync::Arc::new(tokio::sync::Mutex::new(maka_computer_use::Driver::default()));
    let mut run = Session::new(driver.clone());
    let stop = CancellationToken::new();
    stop.cancel();
    assert!(
        run.invoke(
            Command::ListApps {
                options: InventoryOptions::default()
            },
            "cancelled",
            &stop
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("cancelled before dispatch")
    );
    run.close().await.unwrap();
    assert!(
        run.invoke(
            Command::ListApps {
                options: InventoryOptions::default()
            },
            "closed",
            &CancellationToken::new()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("Session is closed")
    );
    run.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an interactive desktop; lists applications without sending input"]
async fn native_application_observation_and_shutdown() {
    let driver = std::sync::Arc::new(tokio::sync::Mutex::new(maka_computer_use::Driver::default()));
    let mut run = Session::new(driver.clone());
    let result = run
        .invoke(
            Command::ListApps {
                options: InventoryOptions::default(),
            },
            "maka-native-smoke",
            &CancellationToken::new(),
        )
        .await;
    let mut other = Session::new(driver.clone());
    let second = other
        .invoke(
            Command::ListApps {
                options: InventoryOptions::default(),
            },
            "maka-native-second",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(second.is_array());
    run.close().await.unwrap();
    let still_live = other
        .invoke(
            Command::ListWindows {
                options: InventoryOptions::default(),
            },
            "maka-native-second",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(still_live.is_array());
    other.close().await.unwrap();
    driver.lock().await.close().await.unwrap();
    let result = result.unwrap();
    assert!(result.is_array());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires a locked macOS desktop; verifies refusal without launching or controlling an app"]
async fn locked_desktop_refuses_target_binding_before_app_resolution() {
    let driver = std::sync::Arc::new(tokio::sync::Mutex::new(maka_computer_use::Driver::default()));
    let mut session = Session::new(driver.clone());
    let error = session
        .invoke(
            Command::GetApp {
                target: maka_computer_use::protocol::AppReference::Window {
                    window_id: u64::MAX,
                },
            },
            "locked-fixture",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("desktop_locked"), "{error}");
    session.close().await.unwrap();
    driver.lock().await.close().await.unwrap();
}
