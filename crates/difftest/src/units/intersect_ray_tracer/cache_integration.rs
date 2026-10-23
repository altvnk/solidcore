//! Native cache insertion regressions; these run only in canonical C++-enabled CI.

use super::*;
use crate::compare::{compare, values_agree};

fn native_and_rust(name: &str, input: &Inputs) -> Result<Vec<f64>, HarnessError> {
    let native =
        ffi::run(&ffi_program(input)).map_err(|e| harness(format!("{name}: native: {e}")))?;
    let rust = run_rust(&Kernel::default(), input, &mut Stats::default())
        .map_err(|e| harness(format!("{name}: Rust: {e}; input={input:?}")))?;
    let diff = compare(
        &Output::new().values("rec", native.clone()),
        &Output::new().values("rec", rust),
        &[],
    );
    assert!(diff.pass(), "{name}: {diff:?}; input={input:?}");
    assert!(diff.values_compared > 0, "{name}: no observations");
    Ok(native)
}

fn sequence_inputs() -> Result<Vec<(String, Inputs)>, HarnessError> {
    #[derive(Deserialize)]
    struct Sequence {
        name: String,
        inputs: Inputs,
    }
    #[derive(Deserialize)]
    struct Sequences {
        cases: Vec<Sequence>,
    }
    let json: Sequences = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/intersect.ray_tracer/cache_sequences.json"
    )))
    .map_err(|e| harness(format!("sequence fixture JSON: {e}")))?;
    Ok(json.cases.into_iter().map(|c| (c.name, c.inputs)).collect())
}

/// Parse the unchanged Fire layout, retaining its full candidate/distance arrays.
fn fire(rec: &[f64], offset: usize) -> Result<(&[f64], usize), HarnessError> {
    let prefix_end = offset
        .checked_add(20)
        .ok_or_else(|| harness("Fire prefix end"))?;
    let prefix = rec
        .get(offset..prefix_end)
        .ok_or_else(|| harness("Fire prefix"))?;
    assert_eq!(prefix.first(), Some(&0.0), "Fire status");
    let count = *prefix
        .get(19)
        .ok_or_else(|| harness("candidate count field"))?;
    let n = count as usize;
    assert_eq!(count, n as f64, "candidate count");
    let end = prefix_end
        .checked_add(
            n.checked_mul(3)
                .ok_or_else(|| harness("Fire array lengths"))?,
        )
        .ok_or_else(|| harness("Fire arrays end"))?;
    Ok((
        rec.get(offset..end).ok_or_else(|| harness("Fire arrays"))?,
        end,
    ))
}

fn assert_hit(rec: &[f64], parameters: [f64; 6], surface: usize) -> Result<(), HarnessError> {
    assert_eq!(
        rec.get(1),
        Some(&1.0),
        "must hit after actual cache insertion"
    );
    // t, u, v, point.x, point.y, point.z (normal and all other fields compare too).
    for (actual, expected) in rec
        .get(6..12)
        .ok_or_else(|| harness("hit parameters"))?
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
    // Candidates describe the final DDA step, not necessarily the step that hit.
    // A later empty step can therefore coexist with this independently checked hit.
    Ok(())
}

#[test]
fn native_cache_sequences_observe_hits_misses_and_explicit_leaf_skips() {
    for (name, input) in sequence_inputs().expect("sequence fixtures") {
        let rec = native_and_rust(&name, &input).expect("native/Rust sequence");
        let inserted = input
            .ops
            .iter()
            .any(|op| matches!(op, OpIn::AddSurface { .. }));
        let offset = usize::from(inserted);
        if inserted {
            assert_eq!(rec[0], 0.0, "{name}: native AddSurfaceToGrid status");
        }
        let (first, next) = fire(&rec, offset).expect("first Fire record");
        match name.as_str() {
            "cached_plane_hit" => {
                assert_hit(first, [1.0, 2.0, 2.0, 2.0, 2.0, 0.0], 0).expect("plane hit");
                let n = first[19] as usize;
                assert!(first[20..20 + n].iter().all(|id| *id == -1.0));
            }
            "cached_bilinear_hit" => {
                assert_hit(first, [0.75, 0.5, 0.5, 2.0, 2.0, 0.25], 0).expect("bilinear hit");
            }
            "cached_then_explicit_leaf_skip" => {
                assert_hit(first, [1.5, 2.0, 2.0, 2.0, 2.0, 0.5], 1).expect("explicit leaf hit");
                let n = first[19] as usize;
                assert!(
                    first[20..20 + n].iter().all(|id| *id == -1.0 || *id == 0.0),
                    "candidate identities"
                );
                let (second, end) = fire(&rec, next).expect("second Fire record");
                assert_hit(second, [2.0, 2.0, 2.0, 2.0, 2.0, 0.0], 0)
                    .expect("cached hit after explicit skip");
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
    assert_eq!(
        native_and_rust(&standalone.name, &input).expect("standalone native/Rust setup"),
        vec![0.0]
    );

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
        let rec = native_and_rust(&name, &input).expect("original generated native/Rust case");
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
