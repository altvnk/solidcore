//! Native cache insertion regressions; these run only in canonical C++-enabled CI.

use super::*;
use crate::compare::{compare, values_agree};

fn native_and_rust(name: &str, input: &Inputs) -> Vec<f64> {
    let native = ffi::run(&ffi_program(input)).unwrap_or_else(|e| panic!("{name}: native: {e}"));
    let rust = run_rust(&Kernel::default(), input, &mut Stats::default())
        .unwrap_or_else(|e| panic!("{name}: Rust: {e}; input={input:?}"));
    let diff = compare(
        &Output::new().values("rec", native.clone()),
        &Output::new().values("rec", rust),
        &[],
    );
    assert!(diff.pass(), "{name}: {diff:?}; input={input:?}");
    assert!(diff.values_compared > 0, "{name}: no observations");
    native
}

fn sequence_inputs() -> Vec<(String, Inputs)> {
    let json: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/intersect.ray_tracer/cache_sequences.json"
    )))
    .expect("sequence fixture JSON");
    json.get("cases")
        .and_then(Value::as_array)
        .expect("cases")
        .iter()
        .map(|c| {
            (
                c.get("name")
                    .and_then(Value::as_str)
                    .expect("name")
                    .to_string(),
                serde_json::from_value(c.get("inputs").expect("inputs field").clone())
                    .expect("inputs"),
            )
        })
        .collect()
}

/// Parse the unchanged Fire layout, retaining its full candidate/distance arrays.
fn fire(rec: &[f64], offset: usize) -> (&[f64], usize) {
    let prefix_end = offset.checked_add(20).expect("Fire prefix end");
    let prefix = rec.get(offset..prefix_end).expect("Fire prefix");
    assert_eq!(prefix.first(), Some(&0.0), "Fire status");
    let count = *prefix.get(19).expect("candidate count field");
    let n = count as usize;
    assert_eq!(count, n as f64, "candidate count");
    let end = prefix_end
        .checked_add(n.checked_mul(3).expect("Fire array lengths"))
        .expect("Fire arrays end");
    (rec.get(offset..end).expect("Fire arrays"), end)
}

fn assert_hit(rec: &[f64], parameters: [f64; 6], surface: usize) {
    assert_eq!(
        rec.get(1),
        Some(&1.0),
        "must hit after actual cache insertion"
    );
    // t, u, v, point.x, point.y, point.z (normal and all other fields compare too).
    for (actual, expected) in rec
        .get(6..12)
        .expect("hit parameters")
        .iter()
        .zip(parameters)
    {
        assert!(
            values_agree(*actual, expected, None),
            "hit field {actual} != {expected}"
        );
    }
    assert_eq!(
        rec.get(16),
        Some(&(surface as f64)),
        "original input surface identity"
    );
    assert!(
        *rec.get(19).expect("candidate count") > 0.0,
        "the hit must traverse actual grid elements"
    );
}

#[test]
fn native_cache_sequences_observe_hits_misses_and_explicit_leaf_skips() {
    for (name, input) in sequence_inputs() {
        let rec = native_and_rust(&name, &input);
        let inserted = input
            .ops
            .iter()
            .any(|op| matches!(op, OpIn::AddSurface { .. }));
        let offset = usize::from(inserted);
        if inserted {
            assert_eq!(rec[0], 0.0, "{name}: native AddSurfaceToGrid status");
        }
        let (first, next) = fire(&rec, offset);
        match name.as_str() {
            "cached_plane_hit" => {
                assert_hit(first, [1.0, 2.0, 2.0, 2.0, 2.0, 0.0], 0);
                let n = first[19] as usize;
                assert!(first[20..20 + n].iter().all(|id| *id == -1.0));
            }
            "cached_bilinear_hit" => {
                assert_hit(first, [0.75, 0.5, 0.5, 2.0, 2.0, 0.25], 0);
            }
            "cached_then_explicit_leaf_skip" => {
                assert_hit(first, [1.5, 2.0, 2.0, 2.0, 2.0, 0.5], 1);
                let n = first[19] as usize;
                assert!(first[20..20 + n].contains(&0.0), "explicit leaf id");
                let (second, end) = fire(&rec, next);
                assert_hit(second, [2.0, 2.0, 2.0, 2.0, 2.0, 0.0], 0);
                assert_eq!(end, rec.len());
                continue;
            }
            "cached_plane_max_t_miss"
            | "cached_bilinear_max_t_miss"
            | "plane_without_insertion_miss"
            | "bilinear_without_insertion_miss" => {
                assert_eq!(first[1], 0.0, "{name}: negative control must miss");
            }
            _ => panic!("unreviewed sequence {name}"),
        }
        assert_eq!(next, rec.len(), "{name}: no output dropped");
    }
}

#[test]
fn six_former_cache_deferrals_compare_real_native_setup_status() {
    let root = crate::case::repo_root();
    let ctx = CaseContext { root, seeds: 0 };
    let fixed = ctx.fixture_cases("intersect.ray_tracer").expect("fixtures");
    let standalone = fixed
        .iter()
        .find(|c| c.name == "add_surface_to_grid_defers")
        .expect("unchanged original standalone fixture");
    let input: Inputs = standalone.parse().expect("standalone input");
    // A setup-status-only comparison, explicitly not credited as a ray-hit gain.
    assert_eq!(native_and_rust(&standalone.name, &input), vec![0.0]);

    for (index, seed) in [
        (56, 9_069_242_292_082_977_168),
        (78, 9_069_242_292_082_977_190),
        (190, 9_069_242_292_082_977_302),
        (266, 9_069_242_292_082_977_378),
        (303, 9_069_242_292_082_977_415),
    ] {
        assert_eq!(CaseContext::seed_for("intersect.ray_tracer", index), seed);
        let value = Runner
            .generate(&mut Rng::new(seed))
            .expect("original generated input");
        let input: Inputs = serde_json::from_value(value).expect("generated input");
        let setups = input
            .ops
            .iter()
            .filter(|op| matches!(op, OpIn::AddSurface { .. }))
            .count();
        let rays = input
            .ops
            .iter()
            .filter(|op| matches!(op, OpIn::Fire { .. }))
            .count();
        assert_eq!(setups, 1, "gen{index:04}: preserve the original setup path");
        let name = format!("gen{index:04}");
        let rec = native_and_rust(&name, &input);
        assert_eq!(
            rec.last(),
            Some(&0.0),
            "{name}: observed final setup status"
        );
        eprintln!(
            "{name}: setup_statuses={setups}, existing_fire_ops={rays}, compared={}",
            rec.len()
        );
    }
}
