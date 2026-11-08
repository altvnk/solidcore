//! Throwaway public-API collector; writes original inputs, complete bits and golden encodings.
use goldens::{CaseContext, RunOptions};
use serde_json::json;
use std::{collections::BTreeMap, io::Write, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let id = args.get(1).ok_or("unit required")?;
    let destination = PathBuf::from(args.get(2).ok_or("output required")?);
    let mut output = std::io::BufWriter::new(std::fs::File::create(destination)?);
    let ctx = CaseContext { root: goldens::case::repo_root(), seeds: 16 };
    let registry = goldens::registry();
    let unit = registry.iter().find(|u| u.id() == id).ok_or("unit missing")?;
    let expected = goldens::golden::read(&goldens::golden::default_dir(), unit.id())?;
    let mut changed = Vec::new();
    let mut count = 0;
    let mut broken = Vec::new();
    goldens::runner::each_rust_output(unit.as_ref(), &ctx, &RunOptions::default(), false, |case, result| {
        count += 1;
        match result {
            Ok(out) => {
                let golden = goldens::golden::encode(&case.name, &out);
                if expected.get(&case.name).is_some_and(|old| goldens::golden::diff(old, &golden, unit.abs_floors()).is_some()) { changed.push(case.name.clone()); }
                let bits: BTreeMap<_, _> = out.values.iter().map(|(key, values)| (key, values.iter().map(|v| v.to_bits()).collect::<Vec<_>>())).collect();
                let values: BTreeMap<_, _> = out.values.iter().map(|(key, values)| (key, values.iter().map(|v| goldens::json::num(*v)).collect::<Vec<_>>())).collect();
                let row = json!({"case":case.name,"inputs":case.inputs,"source":case.source.describe(),"golden":golden,"raw_values":values,"raw_bits":bits,"rejected":out.rejected});
                writeln!(output, "{row}").expect("writing owned verification output");
            }
            Err(error) => broken.push((case.name.clone(), error)),
        }
    })?;
    output.flush()?;
    println!("{}", json!({"unit":id,"cases":count,"changed":changed,"broken":broken}));
    if !broken.is_empty() { return Err("broken cases".into()); }
    Ok(())
}
