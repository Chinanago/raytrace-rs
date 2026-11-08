//! `analysis.budget`: where the launched weight went.
use super::{Analysis, AnalysisRequest, AnalysisResponse, RunView};
use crate::{command::Cmd, engine::EngineError, engine::SurfacePercent, pipeline};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ANALYSIS_BUDGET: Cmd<AnalysisRequest<BudgetSettings>, AnalysisResponse<BudgetOutput>> =
    Cmd::new("analysis.budget");

pub struct Budget;

/// `analysis.budget` has no settings of its own.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BudgetSettings {}

/// One entry of the power budget.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct BudgetEntry {
    /// The weight.
    pub weight: f64,
    /// The weight as a percentage of `reference_weight`.
    pub percent_of_reference: f64,
}

/// The power received by one screen.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct DetectorReceived {
    /// Zero-based index into `report.detectors`, as `analysis.screen` takes it.
    pub index: usize,
    pub name: String,
    /// The integrated power of the screen.
    pub weight: f64,
    pub percent_of_reference: f64,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct BudgetOutput {
    /// The launched weight; every percentage divides by it.
    pub reference_weight: f64,
    pub launched_rays: usize,
    /// Every entry of the budget by field name (`launched`, `detected`, `escaped`,
    /// `geometric_miss`, `surface_absorbed`, `bulk_absorbed`, `numerical_failure`, ...).
    pub entries: BTreeMap<String, BudgetEntry>,
    /// `(accounted - launched) / launched`: how far the entries are from summing up to the
    /// launched weight.
    pub relative_budget_closure_error: f64,
    /// Weight the trace could not resolve, as a percentage of the launched weight:
    /// `numerical_failure + medium_mismatch + branch_cutoff + interaction_limit`. The same
    /// value as `unresolved_percent` of a convergence study.
    pub unresolved_percent_of_reference: f64,
    /// The power received by each screen.
    pub detectors: Vec<DetectorReceived>,
    /// Per-surface encounter totals as percentages of the launched weight.
    pub surfaces: Vec<SurfacePercent>,
}

impl Analysis for Budget {
    const ID: &'static str = "budget";
    const SUMMARY: &'static str =
        "光量収支（基準光量比、収支誤差、未解決分、スクリーンごとの受光、面ごとの到達）を返す";
    type Settings = BudgetSettings;
    type Output = BudgetOutput;

    fn run(view: &RunView<'_>, _: BudgetSettings) -> Result<BudgetOutput, EngineError> {
        let report = view.report();
        let reference = view.reference_weight();
        let percent = pipeline::budget_percent(report, reference)?;
        let entries = pipeline::budget_weights(report)?
            .into_iter()
            .map(|(name, weight)| {
                let percent_of_reference = percent.get(&name).copied().unwrap_or(0.0);
                (
                    name,
                    BudgetEntry {
                        weight,
                        percent_of_reference,
                    },
                )
            })
            .collect();
        Ok(BudgetOutput {
            reference_weight: reference,
            launched_rays: view.calculation().input_count(),
            entries,
            relative_budget_closure_error: pipeline::closure_error(report, reference),
            unresolved_percent_of_reference: pipeline::unresolved_percent(report, reference),
            detectors: report
                .detectors
                .iter()
                .enumerate()
                .map(|(index, map)| DetectorReceived {
                    index,
                    name: map.name.clone(),
                    weight: pipeline::received_weight(map),
                    percent_of_reference: pipeline::received_percent(map, reference),
                })
                .collect(),
            surfaces: SurfacePercent::of_report(report, reference),
        })
    }
}
