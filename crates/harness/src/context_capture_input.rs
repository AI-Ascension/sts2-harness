// SPDX-License-Identifier: MIT

use super::{CaptureBoundary, CaptureComponent, CaptureComponentKind, CapturePort, PreparedInput};

/// Final application-controlled bytes assembled by the Astra bridge.  The bridge passes
/// `stdin` and `output_schema` to the child exactly as provided here; `configuration` is a
/// bounded description of the argv/cwd settings the child sees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedAstraInput<'a> {
    pub stdin: &'a [u8],
    pub output_schema: &'a [u8],
    pub configuration: &'a [u8],
}

impl<'a> PreparedAstraInput<'a> {
    pub const fn new(stdin: &'a [u8], output_schema: &'a [u8], configuration: &'a [u8]) -> Self {
        Self {
            stdin,
            output_schema,
            configuration,
        }
    }

    pub fn capture(
        self,
        capture: &mut dyn CapturePort,
        execution_id: &'a str,
        attempt_id: Option<&'a str>,
    ) {
        if !capture.enabled() {
            return;
        }
        let components = [
            CaptureComponent {
                kind: CaptureComponentKind::Stdin,
                ordinal: 0,
                media_type: "text/plain; charset=utf-8",
                bytes: self.stdin,
            },
            CaptureComponent {
                kind: CaptureComponentKind::OutputSchema,
                ordinal: 1,
                media_type: "application/schema+json",
                bytes: self.output_schema,
            },
            CaptureComponent {
                kind: CaptureComponentKind::Configuration,
                ordinal: 2,
                media_type: "application/json",
                bytes: self.configuration,
            },
        ];
        let _ = capture.prepared_input(PreparedInput {
            execution_id,
            attempt_id,
            boundary: CaptureBoundary::ProviderRequest,
            components: &components,
        });
    }
}

/// Final serialized Ollama body.  Capturing this one opaque component preserves exact JSON byte
/// ordering while the bridge keeps its existing request construction and response parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedOllamaInput<'a> {
    pub body: &'a [u8],
}

impl<'a> PreparedOllamaInput<'a> {
    pub const fn new(body: &'a [u8]) -> Self {
        Self { body }
    }

    pub fn capture(
        self,
        capture: &mut dyn CapturePort,
        execution_id: &'a str,
        attempt_id: Option<&'a str>,
    ) {
        if !capture.enabled() {
            return;
        }
        let component = CaptureComponent {
            kind: CaptureComponentKind::Opaque,
            ordinal: 0,
            media_type: "application/json",
            bytes: self.body,
        };
        let _ = capture.prepared_input(PreparedInput {
            execution_id,
            attempt_id,
            boundary: CaptureBoundary::ProviderRequest,
            components: std::slice::from_ref(&component),
        });
    }
}
