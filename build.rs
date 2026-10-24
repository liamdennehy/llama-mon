//! Build script — downloads Chart.js UMD bundle into the `web/` directory
//! at compile time so it can be embedded via `include_bytes!`.
//!
//! To update the Chart.js version:
//!   1. Change `CHART_JS_VERSION` below
//!   2. Run `cargo clean -p llama-mon` then `cargo build`
//!
//! Chart.js is licensed under the MIT License:
//!   Copyright (c) 2014-2024 Chart.js Contributors
//!   https://github.com/chartjs/Chart.js

use std::fs;
use std::path::Path;

/// Chart.js version to embed. Update this to upgrade.
const CHART_JS_VERSION: &str = "4.4.8";

fn main() {
    let out_dir = env!("CARGO_MANIFEST_DIR");
    let dest = Path::new(out_dir).join("web").join("chart.umd.min.js");

    // Skip download if the file already exists and is non-empty
    if dest.exists() && fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false) {
        println!("chart.js: using existing {}", dest.display());
        return;
    }

    let url = format!(
        "https://cdn.jsdelivr.net/npm/chart.js@{}/dist/chart.umd.min.js",
        CHART_JS_VERSION
    );

    println!("chart.js: downloading {} ...", url);

    let body = reqwest::blocking::get(&url)
        .expect("failed to download chart.js from jsDelivr")
        .text()
        .expect("failed to read chart.js body");

    fs::write(&dest, &body).expect("failed to write chart.umd.min.js");
    println!("chart.js: written {} bytes", body.len());
}
