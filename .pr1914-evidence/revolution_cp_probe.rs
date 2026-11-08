use std::convert::TryFrom;
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};

use serde_json::{json, Map, Value};
use solidcore::curve::curve::Curve;
use solidcore::curve::ellipse::{Ellipse, EllipseOptions};
use solidcore::curve::hyperbola::Hyperbola;
use solidcore::curve::line::Line;
use solidcore::geom::extent1d::Extent1d;
use solidcore::geom::extent2d::Extent2d;
use solidcore::geom::vector3d::Vec3;
use solidcore::kernel::Kernel;
use solidcore::nurbs::b_spline_curve::BSplineCurve;
use solidcore::nurbs::crv_basic::Curve as NurbCurve;
use solidcore::nurbs::math::{CPoint, NOW};
use solidcore::surface::surf_of_revolution::SurfOfRevolution;

type Result<T, E = String> = std::result::Result<T, E>;
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key).ok_or_else(|| format!("missing field {key}"))
}
fn number(v: &Value) -> Result<f64> {
    let x = v.as_f64().ok_or_else(|| format!("expected number, got {v}"))?;
    if !x.is_finite() { return Err("non-finite number".into()); }
    Ok(x)
}
fn array<const N: usize>(v: &Value) -> Result<[f64; N]> {
    let a = v.as_array().ok_or_else(|| format!("expected array, got {v}"))?;
    if a.len() != N { return Err(format!("expected array of length {N}, got {}", a.len())); }
    let mut out = [0.0; N];
    for (dst, src) in out.iter_mut().zip(a) { *dst = number(src)?; }
    Ok(out)
}
fn vec3(v: &Value) -> Result<Vec3> {
    let [x, y, z] = array(v)?;
    Ok(Vec3::new(x, y, z))
}
fn floats(v: &Value) -> Result<Vec<f64>> {
    v.as_array().ok_or_else(|| format!("expected array, got {v}"))?.iter().map(number).collect()
}
fn flag(v: &Value, key: &str) -> Result<bool> {
    match v.get(key) {
        None => Ok(false),
        Some(x) => x.as_bool().ok_or_else(|| format!("{key} must be boolean")),
    }
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    field(v, key)?.as_str().ok_or_else(|| format!("{key} must be a string"))
}
fn bits(x: f64) -> String { format!("{:016x}", x.to_bits()) }
fn input_bits(v: &Value) -> Result<Value> {
    Ok(match v {
        Value::Number(_) => Value::String(bits(number(v)?)),
        Value::Array(a) => Value::Array(a.iter().map(input_bits).collect::<Result<_>>()?),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| Ok((k.clone(), input_bits(v)?))).collect::<Result<Map<String, Value>>>()?),
        _ => v.clone(),
    })
}
fn generator(k: &Kernel, g: &Value) -> Result<Curve> {
    Ok(match text(g, "kind")? {
        "b_spline" => {
            let points = field(g, "cpts")?.as_array().ok_or("cpts must be an array")?;
            let weights = match g.get("weights") {
                None | Some(Value::Null) => None,
                Some(w) => Some(floats(w)?),
            };
            if weights.as_ref().map_or(false, |w| w.len() != points.len()) { return Err("weights: wrong count".into()); }
            let mut pw = Vec::with_capacity(points.len());
            for (i, point) in points.iter().enumerate() {
                let [x, y, z] = array(point)?;
                let (x, y, z, w) = match &weights {
                    None => (x, y, z, NOW),
                    Some(ws) => { let w = ws[i]; (x * w, y * w, z * w, w) }
                };
                pw.push(CPoint::new(x, y, z, w));
            }
            let degree = field(g, "p")?.as_u64().ok_or("p must be a nonnegative integer")?;
            let p = usize::try_from(degree).map_err(|_| "p is too large")?;
            let cur = NurbCurve::new(pw, p, floats(field(g, "u")?)?).map_err(|e| format!("curve input: {e}"))?;
            Curve::from(BSplineCurve::from_nurb(cur, 3, false))
        }
        "line" => {
            let mut line = Line::from_points(vec3(field(g, "a")?)?, vec3(field(g, "b")?)?, 3);
            line.make_nurb().map_err(|e| format!("line nurb: {e}"))?;
            Curve::from(line)
        }
        "circle" => {
            let [d0, d1] = array(field(g, "d")?)?;
            Curve::from(Ellipse::new_circle(k, vec3(field(g, "c")?)?, vec3(field(g, "x")?)?, vec3(field(g, "y")?)?, Extent1d::from_min_max(d0, d1), number(field(g, "r")?)?, &EllipseOptions::default()).map_err(|e| format!("circle: {e}"))?)
        }
        "hyperbola" => {
            let [d0, d1] = array(field(g, "d")?)?;
            Curve::from(Hyperbola::new(vec3(field(g, "c")?)?, vec3(field(g, "x")?)?, vec3(field(g, "y")?)?, Extent1d::from_min_max(d0, d1), number(field(g, "a")?)?, number(field(g, "b")?)?, 3))
        }
        kind => return Err(format!("unknown generator kind {kind}")),
    })
}
fn probe(record: &Value) -> Result<Value> {
    let case = field(record, "case")?;
    let spec = match record.get("spec") {
        Some(s) => s,
        None => field(field(field(field(record, "inputs")?, "off")?, "base")?, "revolution")?,
    };
    let k = Kernel::default();
    let gen = generator(&k, field(spec, "gen")?)?;
    let build = field(spec, "build")?;
    let swap = flag(spec, "swap")?;
    let mut surface = match text(build, "mode")? {
        "canonical" => SurfOfRevolution::create_canonical(&k, gen, vec3(field(build, "point")?)?, vec3(field(build, "axis")?)?, swap).map_err(|e| format!("canonical: {e}"))?,
        "ctor" => {
            let [a, b, c, d] = array(field(build, "domain")?)?;
            let mut s = SurfOfRevolution::new(&k, gen, vec3(field(build, "origin")?)?, vec3(field(build, "x")?)?, vec3(field(build, "y")?)?, Extent2d::from_bounds(a, b, c, d), swap).map_err(|e| format!("constructor: {e}"))?;
            s.make_nurb(&k).map_err(|e| format!("make_nurb: {e}"))?;
            s
        }
        mode => return Err(format!("unknown build mode {mode}")),
    };
    surface.set_out_of_bounds_enabled(flag(spec, "oob")?);
    let net = surface.base().gw_nurb().ok_or("surface has no root NURB")?;
    let ix = net.indices();
    let mut points = Vec::new();
    let mut point_bits = Vec::new();
    for row in net.rows() {
        let mut values = Vec::new();
        let mut values_bits = Vec::new();
        for p in row {
            let xyz = [p.x, p.y, p.z, p.w];
            if xyz.iter().any(|x| !x.is_finite()) { return Err("non-finite control point".into()); }
            values.push(xyz);
            values_bits.push(xyz.map(bits));
        }
        points.push(values);
        point_bits.push(values_bits);
    }
    let u = net.u_knots();
    let v = net.v_knots();
    if u.iter().chain(v.iter()).any(|x| !x.is_finite()) { return Err("non-finite knot".into()); }
    Ok(json!({"case": case, "spec": spec, "input_bits": input_bits(record)?, "verification_only": true,
        "p": ix.p, "q": ix.q, "u_knots": u, "v_knots": v,
        "u_knots_bits": u.iter().map(|&x| bits(x)).collect::<Vec<_>>(),
        "v_knots_bits": v.iter().map(|&x| bits(x)).collect::<Vec<_>>(),
        "control_net": points, "control_net_bits": point_bits}))
}
fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() > 2 { return Err("usage: revolution_cp_probe [input.jsonl [output.jsonl]]".into()); }
    let input: Box<dyn BufRead> = match args.get(0) {
        Some(path) => Box::new(BufReader::new(File::open(path).map_err(|e| format!("input: {e}"))?)),
        None => Box::new(BufReader::new(io::stdin())),
    };
    let mut output: Box<dyn Write> = match args.get(1) {
        Some(path) => Box::new(BufWriter::new(File::create(path).map_err(|e| format!("output: {e}"))?)),
        None => Box::new(BufWriter::new(io::stdout())),
    };
    let mut count = 0;
    for (i, line) in input.lines().enumerate() {
        let line = line.map_err(|e| format!("line {}: {e}", i + 1))?;
        if line.trim().is_empty() { continue; }
        let record: Value = serde_json::from_str(&line).map_err(|e| format!("line {}: {e}", i + 1))?;
        let result = probe(&record).map_err(|e| format!("line {}: {e}", i + 1))?;
        serde_json::to_writer(&mut output, &result).map_err(|e| format!("output: {e}"))?;
        writeln!(output).map_err(|e| format!("output: {e}"))?;
        count += 1;
    }
    if count == 0 { return Err("no input records".into()); }
    output.flush().map_err(|e| format!("output: {e}"))
}
fn main() {
    if let Err(e) = run() { eprintln!("{e}"); std::process::exit(1); }
}
