// SPDX-License-Identifier: MIT

use super::{NativeTransportError, OwnedNativeTransport};
use serde_json::json;

impl OwnedNativeTransport {
    /// Start a bounded native thread through the reviewed method allowlist.
    pub fn start_thread(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("thread/start", json!({}))
    }

    /// Read the bounded native history projection through the reviewed method allowlist.
    pub fn read_thread(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("thread/read", json!({}))
    }

    /// Start a native turn using the adapter-owned request shape.
    pub fn start_turn(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("turn/start", json!({}))
    }

    /// Interrupt a native turn using the adapter-owned request shape.
    pub fn interrupt_turn(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("turn/interrupt", json!({}))
    }

    /// Fork a native thread using the adapter-owned request shape.
    pub fn fork_thread(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("thread/fork", json!({}))
    }

    /// Compact a native thread using the adapter-owned request shape.
    pub fn compact_thread(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("thread/compact", json!({}))
    }

    /// Retire a native thread using the adapter-owned request shape.
    pub fn retire_thread(&mut self) -> Result<serde_json::Value, NativeTransportError> {
        self.request("thread/retire", json!({}))
    }
}
