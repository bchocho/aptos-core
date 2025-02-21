// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

//! # Metrics
//!
//! ## Counters
//!
//! Used to measure values that are added to over time, rates
//! can then be used to check how quickly it changes in graphs.
//! An example would be to add every time an incoming message occurs.
//! ```
//! use prometheus::register_int_counter_vec;
//!
//! register_int_counter_vec!(
//!     "name",
//!     "description",
//!     &["dimension_1", "dimension_2"]
//! );
//! ```
//!
//! ## Gauges
//! Used to measure values that change level over time.  An example
//! would be to set the number of connected peers.
//! ```
//! use prometheus::register_int_gauge_vec;
//!
//! register_int_gauge_vec!(
//!     "name",
//!     "description",
//!     &["dimension_1", "dimension_2"]
//! );
//! ```
//!
//! ## Histograms
//! Used to measure histogram values.  An example is network
//! connection latency.
//! ```
//! use prometheus::register_histogram_vec;
//!
//! register_histogram_vec!(
//!     "name",
//!     "description",
//!     &["dimension_1", "dimension_2"]
//! );
//! ```

#![forbid(unsafe_code)]

mod json_encoder;
pub mod metric_server;
pub mod system_information;
pub mod system_metrics;

pub use aptos_metrics_core::op_counters::{DurationHistogram, OpMetrics};

#[cfg(test)]
mod unit_tests;

use aptos_logger::prelude::*;
use aptos_metrics_core::{register_int_counter_vec, IntCounterVec};
use once_cell::sync::Lazy;

pub static NUM_METRICS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "aptos_metrics",
        "Number of metrics in certain states",
        &["type"]
    )
    .unwrap()
});

pub fn gather_metrics() -> Vec<prometheus::proto::MetricFamily> {
    let metric_families = aptos_metrics_core::gather();
    let mut total: u64 = 0;
    let mut families_over_1000: u64 = 0;

    // Take metrics of metric gathering so we know possible overhead of this process
    for metric_family in &metric_families {
        let family_count = metric_family.get_metric().len();
        if family_count > 1000 {
            families_over_1000 = families_over_1000.saturating_add(1);
            let name = metric_family.get_name();
            warn!(
                count = family_count,
                metric_family = name,
                "Metric Family '{}' over 1000 dimensions '{}'",
                name,
                family_count
            );
        }
        total = total.saturating_add(family_count as u64);
    }

    // These metrics will be reported on the next pull, rather than create a new family
    NUM_METRICS.with_label_values(&["total"]).inc_by(total);
    NUM_METRICS
        .with_label_values(&["families_over_1000"])
        .inc_by(families_over_1000);

    metric_families
}
