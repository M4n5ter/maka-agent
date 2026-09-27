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

//! Retained AX elements are the identity for macOS semantic operations. Cua's
//! bounded walker and exact-window checks supply the platform implementation.
pub(crate) mod pasteboard;
use crate::protocol::*;
use core_foundation::{
    base::{CFEqual, CFGetTypeID, CFRange, CFRelease, TCFType},
    string::CFString,
};
use cua_driver_core::{
    background_input::{
        BackgroundAction, BackgroundInputDecision, ExactWindowTarget, decide_background_input,
    },
    walk_budget::WalkBudget,
};
use maka_runtime::tools::ToolError;
use platform_macos::ax::{
    bindings::*,
    cache::RetainedElement,
    exact_target::{element_window_id, gather_background_facts},
    tree::walk_tree_budgeted,
};
use std::collections::HashMap;

unsafe extern "C" {
    fn AXValueGetTypeID() -> core_foundation::base::CFTypeID;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> core_foundation::dictionary::CFDictionaryRef;
}

pub(crate) fn require_desktop() -> Result<(), ToolError> {
    use core_foundation::{
        boolean::{CFBooleanGetTypeID, kCFBooleanTrue},
        dictionary::CFDictionaryGetValue,
    };
    // SAFETY: the copied session dictionary and its borrowed values retain
    // their CF types until the dictionary is released below.
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return Err(failed("desktop_unavailable: no interactive macOS session"));
        }
        let key = CFString::new("CGSSessionScreenIsLocked");
        let value = CFDictionaryGetValue(session, key.as_concrete_TypeRef().cast());
        let locked = !value.is_null()
            && CFGetTypeID(value) == CFBooleanGetTypeID()
            && value == kCFBooleanTrue.cast();
        CFRelease(session.cast());
        if locked {
            return Err(failed(
                "desktop_locked: unlock the macOS desktop before native Computer Use",
            ));
        }
    }
    Ok(())
}

pub(crate) struct Target {
    pid: i32,
    window_id: u32,
    window: RetainedElement,
    elements: HashMap<u64, RetainedElement>,
}
impl Target {
    pub(crate) fn geometry(&self) -> Result<cua_driver_contract::WindowBounds, ToolError> {
        let [x, y, width, height] =
            unsafe { element_screen_rect(self.window.as_ptr() as AXUIElementRef) }
                .ok_or_else(|| failed("AX window geometry unavailable"))?;
        Ok(cua_driver_contract::WindowBounds {
            x,
            y,
            width,
            height,
        })
    }
    pub(crate) fn paste(&self, text: &str, html: bool) -> Result<bool, ToolError> {
        self.verify()?;
        let pointer = unsafe {
            platform_macos::ax::exact_target::focused_element_in_window(self.pid, self.window_id)
        }
        .ok_or_else(|| failed("paste requires a focused element in the bound window"))?;
        let element = unsafe {
            let retained = RetainedElement::retain(pointer as usize);
            CFRelease(pointer.cast());
            retained
        };
        let pointer = element.as_ptr() as AXUIElementRef;
        let before = unsafe { copy_string_attr(pointer, "AXValue") }.ok_or_else(|| {
            failed("paste requires an observable editable value; use supported element actions")
        })?;
        let selection = unsafe { read_range(pointer) }.ok_or_else(|| {
            failed("paste requires an observable selection; use typeText or browser tab input")
        })?;
        let restore = pasteboard::replace(text, html)?;
        let posted = platform_macos::input::skylight::with_foreground_hid_activation(
            self.pid,
            self.window_id,
            || {
                // Activation can reset the first responder/selection. Reapply the
                // exact retained field and range inside the foreground interval.
                unsafe {
                    if set_bool_attr_true(pointer, "AXFocused") != kAXErrorSuccess {
                        return Err(
                            std::io::Error::other("paste field could not be focused").into()
                        );
                    }
                    write_range(pointer, selection)
                        .map_err(|e| std::io::Error::other(e.to_string()))?;
                }
                platform_macos::input::keyboard::press_key_global("v", &["cmd"])
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut consumed = false;
        while std::time::Instant::now() < deadline {
            if unsafe { copy_string_attr(pointer, "AXValue") }.is_some_and(|after| after != before)
            {
                consumed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let restored = restore.restore()?;
        posted.map_err(|e| ToolError::OutcomeUnknown(e.to_string()))?;
        if !consumed {
            return Err(ToolError::OutcomeUnknown("paste did not produce an observable change; inspect the UI before trying another action".into()));
        }
        Ok(restored)
    }
    pub(crate) fn bind(pid: u32, window_id: u64) -> Result<Self, ToolError> {
        let pid = i32::try_from(pid).map_err(failed)?;
        let window_id = u32::try_from(window_id).map_err(failed)?;
        // SAFETY: all copied AX references are released or transferred to an
        // owned RetainedElement. No reference is reconstructed from model data.
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return Err(failed("AX application unavailable"));
            }
            AXUIElementSetMessagingTimeout(app, 0.25);
            platform_macos::ax::enablement::ensure_chromium_ax_enabled(pid, app);
            let windows = copy_ax_windows_including(app, pid, window_id);
            CFRelease(app.cast());
            let mut selected = None;
            for window in windows {
                if ax_get_window_id(window) == Some(window_id) {
                    selected = Some(RetainedElement::retain(window as usize));
                }
                CFRelease(window.cast());
            }
            let window = selected.ok_or_else(|| failed("AX window unavailable"))?;
            Ok(Self {
                pid,
                window_id,
                window,
                elements: HashMap::new(),
            })
        }
    }
    pub(crate) fn verify(&self) -> Result<(), ToolError> {
        let fresh = Self::bind(self.pid as u32, self.window_id as u64)?;
        // CF equality compares the remote AX identity, not the wrapper pointer.
        if unsafe { CFEqual(self.window.as_ptr() as _, fresh.window.as_ptr() as _) } == 0 {
            return Err(failed("stale_target: window was replaced; bind it again"));
        }
        Ok(())
    }
    pub(crate) fn clear(&mut self) {
        self.elements.clear();
    }
    pub(crate) fn observe(&mut self) -> Result<String, ToolError> {
        self.elements.clear();
        self.verify()?;
        let tree = walk_tree_budgeted(
            self.pid,
            Some(self.window_id),
            None,
            30,
            WalkBudget::new(1500, 2000),
        );
        if !tree
            .window_scope
            .as_ref()
            .is_some_and(|scope| scope.is_matched())
        {
            return Err(failed("AX window did not resolve"));
        }
        let mut lines = Vec::new();
        for node in tree.nodes {
            let index = if let Some(index) = node.element_index {
                // The Cua walker transfers a retained reference for indexed
                // nodes. Own one retain and release the transferred reference.
                unsafe {
                    let element = RetainedElement::retain(node.element_ptr);
                    CFRelease(node.element_ptr as _);
                    self.elements.insert(index as u64, element);
                }
                format!("{index} ")
            } else {
                String::new()
            };
            let name = node
                .title
                .filter(|s| !s.is_empty())
                .or(node.description)
                .unwrap_or_default();
            let value = node.value_state.or(node.value).unwrap_or_default();
            lines.push(format!(
                "{}{}{} {:?}{}{}",
                "  ".repeat(node.depth.min(30)),
                index,
                node.role,
                name,
                if value.is_empty() {
                    String::new()
                } else {
                    format!(" value={value:?}")
                },
                if node.actions.is_empty() {
                    String::new()
                } else {
                    format!(" actions={:?}", node.actions)
                }
            ));
        }
        self.verify()?;
        if tree.truncated {
            lines.push("Accessibility tree is incomplete (walk budget reached).".into());
        }
        Ok(lines.join("\n"))
    }
    pub(crate) fn handles(action: &Action) -> bool {
        matches!(
            action,
            Action::SetValue { .. }
                | Action::SelectText { .. }
                | Action::Secondary { .. }
                | Action::Click {
                    target: Position::Element(_),
                    ..
                }
                | Action::Scroll {
                    target: Position::Element(_),
                    ..
                }
        )
    }
    fn element(&self, index: u64) -> Result<AXUIElementRef, ToolError> {
        self.verify()?;
        let element = self
            .elements
            .get(&index)
            .ok_or_else(|| failed("stale_element: read getAXState first"))?
            .as_ptr() as AXUIElementRef;
        // SAFETY: `elements` retains this AX object through the whole operation.
        let facts = gather_background_facts(self.pid, self.window_id, Some(element as usize));
        if let BackgroundInputDecision::Refuse(refusal) = decide_background_input(
            ExactWindowTarget {
                pid: self.pid,
                window_id: self.window_id,
            },
            &facts,
            BackgroundAction::AxSemantic,
        ) {
            return Err(failed(format!("{}: {}", refusal.code, refusal.reason)));
        }
        if unsafe { element_window_id(element) } != Some(self.window_id) {
            return Err(failed("element no longer belongs to this window"));
        }
        if unsafe { copy_bool_attr(element, "AXEnabled") } == Some(false) {
            return Err(failed("element is disabled"));
        }
        Ok(element)
    }
    pub(crate) fn action(&mut self, action: Action) -> Result<(), ToolError> {
        // SAFETY: each pointer comes from `element`, which rechecks the retained
        // target and ancestry; the Session serializes actions and retirement.
        unsafe {
            match action {
                Action::Click {
                    target: Position::Element(index),
                    options,
                } => {
                    if options.click_count.unwrap_or(1) != 1 {
                        return Err(failed(
                            "native semantic clicks support one click; use screenshot coordinates for multiple clicks",
                        ));
                    }
                    let element = self.element(index)?;
                    let actions = copy_action_names(element);
                    let action = match options.mouse_button.unwrap_or(MouseButton::Left) {
                        MouseButton::Left => "AXPress",
                        MouseButton::Right => "AXShowMenu",
                        MouseButton::Middle => {
                            return Err(failed("native semantic middle click unavailable"));
                        }
                    };
                    if actions.iter().any(|a| a == action) {
                        check(perform_action(element, action))?;
                    } else if action == "AXPress" && is_attribute_settable(element, "AXFocused") {
                        check(set_bool_attr_true(element, "AXFocused"))?;
                    } else {
                        return Err(failed(
                            "element has no requested click action; use its screenshot geometry",
                        ));
                    }
                }
                Action::Secondary { index, action } => {
                    let element = self.element(index)?;
                    if !copy_action_names(element).contains(&action) {
                        return Err(failed(
                            "action_not_exposed: use an action listed in the current AX state",
                        ));
                    }
                    check(perform_action(element, &action))?;
                }
                Action::SetValue { index, value } => {
                    let element = self.element(index)?;
                    if !is_attribute_settable(element, "AXValue") {
                        return Err(failed(
                            "AXValue is not settable; use the element's input or selection actions",
                        ));
                    }
                    if copy_number_attr(element, "AXValue").is_some() {
                        let number: f64 = value.parse().map_err(failed)?;
                        if !number.is_finite() {
                            return Err(failed("value must be finite"));
                        }
                        check(set_number_attr(element, "AXValue", number))?;
                        if copy_number_attr(element, "AXValue") != Some(number) {
                            return Err(failed("AX value read-back did not match"));
                        }
                    } else {
                        check(set_string_attr(element, "AXValue", &value))?;
                        if copy_string_attr(element, "AXValue").as_deref() != Some(&value) {
                            return Err(failed("AX value read-back did not match"));
                        }
                    }
                }
                Action::SelectText {
                    index,
                    text,
                    options,
                } => {
                    let element = self.element(index)?;
                    if !is_attribute_settable(element, "AXSelectedTextRange") {
                        return Err(failed(
                            "text range selection is unavailable on this element",
                        ));
                    }
                    let value = copy_string_attr(element, "AXValue")
                        .ok_or_else(|| failed("editable text unavailable"))?;
                    let (start, length) = selection(&value, &text, &options)?;
                    let range = (start as isize, length as isize);
                    write_range(element, range)?;
                    if read_range(element) != Some(range) {
                        return Err(failed("AX selection read-back failed"));
                    }
                }
                Action::Scroll {
                    target: Position::Element(index),
                    direction,
                    distance,
                } => {
                    let pages = match distance {
                        None => 1,
                        Some(Distance::Pages(pages)) => pages,
                        _ => return Err(failed("native semantic scroll accepts page counts")),
                    };
                    if !(1..=50).contains(&pages) {
                        return Err(failed("scroll page count out of range"));
                    }
                    let action = match direction {
                        Direction::Up => "AXScrollUpByPage",
                        Direction::Down => "AXScrollDownByPage",
                        Direction::Left => "AXScrollLeftByPage",
                        Direction::Right => "AXScrollRightByPage",
                    };
                    for _ in 0..pages {
                        let element = self.element(index)?;
                        if !copy_action_names(element).iter().any(|a| a == action) {
                            return Err(failed(
                                "element exposes no semantic page-scroll action; use screenshot coordinates",
                            ));
                        }
                        check(perform_action(element, action))?;
                    }
                }
                _ => return Err(failed("unsupported AX action")),
            }
        }
        Ok(())
    }
}
unsafe fn read_range(element: AXUIElementRef) -> Option<(isize, isize)> {
    let attribute = CFString::new("AXSelectedTextRange");
    let mut value = std::ptr::null();
    if unsafe {
        AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
    } != kAXErrorSuccess
        || value.is_null()
    {
        return None;
    }
    if unsafe { CFGetTypeID(value) != AXValueGetTypeID() } {
        unsafe { CFRelease(value) };
        return None;
    }
    let mut range = CFRange::init(0, 0);
    let valid = unsafe {
        AXValueGetValue(
            value as _,
            kAXValueCFRangeType,
            (&mut range as *mut CFRange).cast(),
        )
    };
    unsafe { CFRelease(value) };
    valid.then_some((range.location, range.length))
}
unsafe fn write_range(element: AXUIElementRef, range: (isize, isize)) -> Result<(), ToolError> {
    let range = CFRange::init(range.0, range.1);
    let value = unsafe { AXValueCreate(kAXValueCFRangeType, (&range as *const CFRange).cast()) };
    if value.is_null() {
        return Err(failed("AX range allocation failed"));
    }
    let attribute = CFString::new("AXSelectedTextRange");
    let result = unsafe {
        AXUIElementSetAttributeValue(element, attribute.as_concrete_TypeRef(), value.cast())
    };
    unsafe { CFRelease(value.cast()) };
    check(result)
}
fn selection(
    value: &str,
    text: &str,
    options: &SelectOptions,
) -> Result<(usize, usize), ToolError> {
    if text.is_empty() {
        return Err(failed("selectText requires nonempty text"));
    }
    let matches: Vec<_> = value
        .char_indices()
        .filter(|(at, _)| {
            value[*at..].starts_with(text)
                && options
                    .prefix
                    .as_ref()
                    .is_none_or(|p| value[..*at].ends_with(p))
                && options
                    .suffix
                    .as_ref()
                    .is_none_or(|s| value[*at + text.len()..].starts_with(s))
        })
        .map(|(at, _)| at)
        .collect();
    if matches.len() != 1 {
        return Err(failed(
            "text match is missing or ambiguous; provide prefix/suffix",
        ));
    }
    let start = value[..matches[0]].encode_utf16().count();
    let length = text.encode_utf16().count();
    Ok(match options.selection_type {
        None | Some(SelectionType::Text) => (start, length),
        Some(SelectionType::CursorBefore) => (start, 0),
        Some(SelectionType::CursorAfter) => (start + length, 0),
    })
}
fn check(status: i32) -> Result<(), ToolError> {
    if status == kAXErrorSuccess {
        Ok(())
    } else {
        Err(failed(format!("AX operation refused ({status})")))
    }
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_selection_disambiguates_overlaps_and_uses_utf16_offsets() {
        let options = SelectOptions {
            prefix: Some("😀 hello ".into()),
            suffix: None,
            selection_type: Some(SelectionType::Text),
        };
        assert_eq!(
            selection("😀 hello hello!", "hello", &options).unwrap(),
            (9, 5)
        );
        assert!(selection("aaa", "aa", &SelectOptions::default()).is_err());
        assert!(selection("abc", "", &SelectOptions::default()).is_err());
    }
}
