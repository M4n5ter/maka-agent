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

//! Root-scoped native reminders. Delivery acknowledgement follows a visible frame.
mod view;
use maka_client::notifications::{Notice, NotificationDelivery};
use std::collections::BTreeMap;
pub(crate) use view::{input, paint, sheet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub package: String,
    pub id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    Close,
    Read(Key),
    Dismiss(Key),
    Select(Key),
    Back,
    Page(usize),
    Copy,
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open => "attention-title",
            Self::Close => "session-remove-close",
            Self::Read(_) => "attention-read",
            Self::Dismiss(_) => "attention-dismiss",
            Self::Select(_) => "attention-open",
            Self::Back => "extensions-back",
            Self::Page(_) => "resources-more",
            Self::Copy => "chat-copy-message",
        }
    }
}
struct Item {
    delivery: NotificationDelivery,
    duplicates: Vec<NotificationDelivery>,
    read: bool,
    presented: bool,
    order: u64,
}
#[derive(Default)]
pub struct State {
    pub visible: bool,
    root: Option<String>,
    items: BTreeMap<Key, Item>,
    full: bool,
    order: u64,
    offset: usize,
    selected: Option<Key>,
    scroll: u16,
    copy: Option<String>,
}
impl State {
    pub fn unread(&self) -> usize {
        self.items.values().filter(|item| !item.read).count()
    }
    pub fn receive(&mut self, root: &str, delivery: NotificationDelivery) -> bool {
        if delivery.is_cancelled() {
            return false;
        }
        if self.root.as_deref() != Some(root) {
            self.items.clear();
            self.root = Some(root.into());
            self.full = false;
            self.offset = 0;
            self.order = 0;
            self.selected = None;
            self.copy = None;
        }
        let key = Key {
            package: delivery.notice.package_id.clone(),
            id: delivery.notice.notification.id.clone(),
        };
        if let Some(item) = self.items.get_mut(&key) {
            if item.delivery.notice != delivery.notice {
                return false;
            }
            let mut delivery = delivery;
            if item.presented {
                delivery.acknowledge_presented();
            } else {
                item.duplicates.push(delivery);
            }
            return true;
        }
        let cost = |notice: &Notice| {
            notice.package_id.len()
                + notice.notification.id.len()
                + notice.notification.title.len()
                + notice.notification.body.len()
                + 512
        };
        let retained: usize = self
            .items
            .values()
            .map(|item| cost(&item.delivery.notice))
            .sum();
        if retained + cost(&delivery.notice) > 4 * 1024 * 1024 {
            self.full = true;
            return false;
        }
        self.order += 1;
        self.items.insert(
            key,
            Item {
                delivery,
                duplicates: vec![],
                read: false,
                presented: false,
                order: self.order,
            },
        );
        true
    }
    /// Root calls this only after drawing the visible attention entry or inbox.
    pub fn presented(&mut self, visible: bool) {
        if !visible {
            return;
        }
        self.items
            .retain(|_, item| item.presented || !item.delivery.is_cancelled());
        for item in self.items.values_mut() {
            if !item.presented {
                item.delivery.acknowledge_presented();
                for duplicate in &mut item.duplicates {
                    duplicate.acknowledge_presented();
                }
                item.duplicates.clear();
                item.presented = true;
            }
        }
    }
    pub fn disconnect(&mut self) {
        // Acknowledged Root-local history remains readable in memory. Pending
        // deliveries must not outlive their connection or manufacture an ack.
        self.items.retain(|_, item| item.presented);
        for item in self.items.values_mut() {
            item.duplicates.clear();
        }
    }
    pub fn take_copy(&mut self) -> Option<String> {
        self.copy.take()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn enabled(&self, command: &Command) -> bool {
        match command {
            Command::Open => true,
            Command::Close | Command::Back => self.visible,
            Command::Page(offset) => self.visible && *offset < self.items.len(),
            Command::Copy => {
                self.visible
                    && self
                        .selected
                        .as_ref()
                        .is_some_and(|key| self.items.contains_key(key))
            }
            Command::Read(key) | Command::Dismiss(key) | Command::Select(key) => {
                self.visible && self.items.contains_key(key)
            }
        }
    }
    pub fn action(&mut self, command: Command) {
        if !self.enabled(&command) {
            return;
        }
        match command {
            Command::Open => {
                self.visible = true;
                self.offset = 0;
            }
            Command::Close => self.visible = false,
            Command::Back => self.selected = None,
            Command::Page(offset) => self.offset = offset,
            Command::Select(key) => {
                if let Some(item) = self.items.get_mut(&key) {
                    item.read = true;
                }
                self.selected = Some(key);
                self.scroll = 0;
            }
            Command::Copy => {
                self.copy = self
                    .selected
                    .as_ref()
                    .and_then(|key| self.items.get(key))
                    .map(|item| item.delivery.notice.notification.body.clone());
            }
            Command::Read(key) => {
                if let Some(item) = self.items.get_mut(&key) {
                    item.read = true;
                }
            }
            Command::Dismiss(key) => {
                self.items.remove(&key);
                if self.selected.as_ref() == Some(&key) {
                    self.selected = None;
                }
                self.offset = self.offset.min(self.items.len().saturating_sub(1));
                self.full = false;
            }
        }
    }
}
