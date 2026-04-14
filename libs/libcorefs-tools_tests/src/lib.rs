// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Host-side integration tests for `libcorefs-tools`.
//!
//! The real unit tests live alongside the production code in
//! [`libcorefs_tools`] (enabled via `#[cfg(test)]`).  This sibling
//! crate exists purely to detach the test build from the workspace's
//! `build-std` requirement: it declares its own `[workspace]` so
//! `cargo test` at this manifest compiles with the regular host
//! target and stdlib.
//!
//! The tests below re-verify a handful of public-API invariants that
//! are worth asserting at the crate boundary (integration-level).

#![allow(clippy::collapsible_if)]

#[cfg(test)]
mod integration {
    use corefs_core::error::CoreFsError;
    use libcorefs_tools::args::{parse, parse_capacity, parse_device_id, parse_u64};
    use libcorefs_tools::error::{exit_code_for, ExitCode};
    use libcorefs_tools::report::{JsonBuilder, Report};

    struct DummyReport;
    impl Report for DummyReport {
        fn summary(&self) -> String {
            "dummy ok".into()
        }
        fn render_text(&self) -> String {
            "dummy\n─────\nstatus : ok\n".into()
        }
        fn render_json(&self) -> String {
            let mut b = JsonBuilder::new();
            b.begin_object();
            b.kv_string("status", "ok");
            b.end_object();
            b.finish()
        }
    }

    #[test]
    fn end_to_end_arg_parsing() {
        let a = parse("--device 2 --capacity 64M --json");
        assert_eq!(parse_device_id(&a), Some(2));
        assert_eq!(parse_capacity(&a), Some(64 * 1024 * 1024));
        assert!(a.has("json"));
    }

    #[test]
    fn exit_code_mapping_is_stable() {
        assert_eq!(
            exit_code_for(&CoreFsError::NotFound("x".into())),
            ExitCode::NotFound
        );
        assert_eq!(
            exit_code_for(&CoreFsError::InvalidInput("x".into())),
            ExitCode::InvalidArgument
        );
        assert_eq!(ExitCode::Success.as_u32(), 0);
    }

    #[test]
    fn report_has_summary_and_json() {
        let r = DummyReport;
        assert_eq!(r.summary(), "dummy ok");
        assert!(r.render_json().starts_with('{'));
    }

    #[test]
    fn json_builder_nesting() {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.key("a");
        b.begin_array();
        b.u64(1);
        b.u64(2);
        b.u64(3);
        b.end_array();
        b.end_object();
        assert_eq!(b.finish(), r#"{"a":[1,2,3]}"#);
    }

    #[test]
    fn parse_u64_rejects_garbage() {
        assert_eq!(parse_u64("abc"), None);
        assert_eq!(parse_u64(""), None);
        assert_eq!(parse_u64("12x"), None);
    }
}
