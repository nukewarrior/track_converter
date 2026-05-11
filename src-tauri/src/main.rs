use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use xmltree::{Element, EmitterConfig, XMLNode};

const A: f64 = 6378245.0;
const EE: f64 = 0.006693421622965943;
const MIN_LON: f64 = 72.004;
const MAX_LON: f64 = 137.8347;
const MIN_LAT: f64 = 0.8293;
const MAX_LAT: f64 = 55.8271;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchRequest {
    files: Vec<String>,
    input_format: String,
    output_format: String,
    mode: String,
    csv_lat_column: Option<String>,
    csv_lon_column: Option<String>,
    csv_no_header: bool,
}

#[derive(Debug, Serialize)]
struct BatchResult {
    ok: usize,
    skipped: usize,
    failed: usize,
    logs: Vec<ConvertLog>,
}

#[derive(Debug, Serialize)]
struct ConvertLog {
    input: String,
    output: Option<String>,
    status: String,
    message: String,
    points: usize,
}

#[derive(Clone, Debug)]
struct TrackPoint {
    lon: f64,
    lat: f64,
    name: Option<String>,
}

#[tauri::command]
fn convert_files(request: BatchRequest) -> Result<BatchResult, String> {
    let mut result = BatchResult {
        ok: 0,
        skipped: 0,
        failed: 0,
        logs: Vec::new(),
    };

    if request.files.is_empty() {
        return Err("请先添加 CSV、GPX 或 KML 文件".to_string());
    }

    for file in &request.files {
        match convert_one(file, &request) {
            Ok(log) if log.status == "ok" => {
                result.ok += 1;
                result.logs.push(log);
            }
            Ok(log) if log.status == "skipped" => {
                result.skipped += 1;
                result.logs.push(log);
            }
            Ok(log) => {
                result.failed += 1;
                result.logs.push(log);
            }
            Err(message) => {
                result.failed += 1;
                result.logs.push(ConvertLog {
                    input: file.clone(),
                    output: None,
                    status: "failed".to_string(),
                    message,
                    points: 0,
                });
            }
        }
    }

    Ok(result)
}

fn convert_one(file: &str, request: &BatchRequest) -> Result<ConvertLog, String> {
    let input_path = PathBuf::from(file);
    if !input_path.exists() {
        return Err("文件不存在".to_string());
    }

    let input_format = detect_format(&input_path, &request.input_format)?;
    let output_format = normalized_output_format(&request.output_format)?;

    if input_format == "gdb" {
        return Ok(ConvertLog {
            input: file.to_string(),
            output: None,
            status: "skipped".to_string(),
            message: "GDB 已预留，当前版本暂不转换".to_string(),
            points: 0,
        });
    }

    let output_path = make_output_path(&input_path, output_format, &request.mode)?;
    let transform = transform_for_mode(&request.mode)?;

    let points = if input_format == output_format {
        match input_format.as_str() {
            "csv" => convert_csv_preserving(&input_path, &output_path, transform, request)?,
            "gpx" => convert_gpx_preserving(&input_path, &output_path, transform)?,
            "kml" => convert_kml_preserving(&input_path, &output_path, transform)?,
            _ => return Err(format!("暂不支持输入格式：{input_format}")),
        }
    } else {
        let points = extract_points(&input_path, &input_format, transform, request)?;
        write_points(&output_path, output_format, &points)?;
        points.len()
    };

    Ok(ConvertLog {
        input: file.to_string(),
        output: Some(output_path.to_string_lossy().to_string()),
        status: "ok".to_string(),
        message: format!("已转换 {points} 个点位"),
        points,
    })
}

fn convert_csv_preserving(
    input_path: &Path,
    output_path: &Path,
    transform: fn(f64, f64) -> (f64, f64),
    request: &BatchRequest,
) -> Result<usize, String> {
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .has_headers(!request.csv_no_header)
        .from_path(input_path)
        .map_err(|err| err.to_string())?;
    let mut writer = WriterBuilder::new()
        .from_path(output_path)
        .map_err(|err| err.to_string())?;

    let headers = if request.csv_no_header {
        None
    } else {
        let header = reader.headers().map_err(|err| err.to_string())?.clone();
        writer
            .write_record(&header)
            .map_err(|err| err.to_string())?;
        Some(header.iter().map(ToString::to_string).collect::<Vec<_>>())
    };

    let (lat_index, lon_index) = csv_column_indexes(headers.as_deref(), request)?;
    let mut count = 0;

    for record in reader.records() {
        let record = record.map_err(|err| err.to_string())?;
        let mut fields = record.iter().map(ToString::to_string).collect::<Vec<_>>();
        if fields.len() <= lat_index.max(lon_index) {
            writer.write_record(&fields).map_err(|err| err.to_string())?;
            continue;
        }

        let lat = parse_float(&fields[lat_index], "CSV 纬度")?;
        let lon = parse_float(&fields[lon_index], "CSV 经度")?;
        let (new_lon, new_lat) = transform(lon, lat);
        fields[lon_index] = format_float(new_lon);
        fields[lat_index] = format_float(new_lat);
        writer.write_record(&fields).map_err(|err| err.to_string())?;
        count += 1;
    }

    writer.flush().map_err(|err| err.to_string())?;
    Ok(count)
}

fn convert_gpx_preserving(
    input_path: &Path,
    output_path: &Path,
    transform: fn(f64, f64) -> (f64, f64),
) -> Result<usize, String> {
    let mut root = read_xml(input_path)?;
    let count = transform_gpx_element(&mut root, transform)?;
    write_xml(output_path, &root)?;
    Ok(count)
}

fn convert_kml_preserving(
    input_path: &Path,
    output_path: &Path,
    transform: fn(f64, f64) -> (f64, f64),
) -> Result<usize, String> {
    let mut root = read_xml(input_path)?;
    let count = transform_kml_element(&mut root, transform)?;
    write_xml(output_path, &root)?;
    Ok(count)
}

fn extract_points(
    input_path: &Path,
    input_format: &str,
    transform: fn(f64, f64) -> (f64, f64),
    request: &BatchRequest,
) -> Result<Vec<TrackPoint>, String> {
    match input_format {
        "csv" => extract_csv_points(input_path, transform, request),
        "gpx" => {
            let root = read_xml(input_path)?;
            let mut points = Vec::new();
            collect_gpx_points(&root, transform, &mut points)?;
            Ok(points)
        }
        "kml" => {
            let root = read_xml(input_path)?;
            let mut points = Vec::new();
            collect_kml_points(&root, transform, &mut points)?;
            Ok(points)
        }
        _ => Err(format!("暂不支持输入格式：{input_format}")),
    }
}

fn extract_csv_points(
    input_path: &Path,
    transform: fn(f64, f64) -> (f64, f64),
    request: &BatchRequest,
) -> Result<Vec<TrackPoint>, String> {
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .has_headers(!request.csv_no_header)
        .from_path(input_path)
        .map_err(|err| err.to_string())?;
    let headers = if request.csv_no_header {
        None
    } else {
        Some(
            reader
                .headers()
                .map_err(|err| err.to_string())?
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )
    };
    let (lat_index, lon_index) = csv_column_indexes(headers.as_deref(), request)?;
    let mut points = Vec::new();

    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|err| err.to_string())?;
        if record.len() <= lat_index.max(lon_index) {
            continue;
        }
        let lat = parse_float(&record[lat_index], "CSV 纬度")?;
        let lon = parse_float(&record[lon_index], "CSV 经度")?;
        let (new_lon, new_lat) = transform(lon, lat);
        points.push(TrackPoint {
            lon: new_lon,
            lat: new_lat,
            name: Some(format!("point-{}", index + 1)),
        });
    }

    Ok(points)
}

fn write_points(output_path: &Path, output_format: &str, points: &[TrackPoint]) -> Result<(), String> {
    match output_format {
        "csv" => write_points_csv(output_path, points),
        "gpx" => write_points_gpx(output_path, points),
        "kml" => write_points_kml(output_path, points),
        _ => Err(format!("暂不支持输出格式：{output_format}")),
    }
}

fn write_points_csv(output_path: &Path, points: &[TrackPoint]) -> Result<(), String> {
    let mut writer = WriterBuilder::new()
        .from_path(output_path)
        .map_err(|err| err.to_string())?;
    writer
        .write_record(["name", "lon", "lat"])
        .map_err(|err| err.to_string())?;

    for (index, point) in points.iter().enumerate() {
        let name = point
            .name
            .clone()
            .unwrap_or_else(|| format!("point-{}", index + 1));
        writer
            .write_record([name, format_float(point.lon), format_float(point.lat)])
            .map_err(|err| err.to_string())?;
    }

    writer.flush().map_err(|err| err.to_string())
}

fn write_points_gpx(output_path: &Path, points: &[TrackPoint]) -> Result<(), String> {
    let mut text = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="Track Converter" xmlns="http://www.topografix.com/GPX/1/1">
  <trk>
    <name>converted</name>
    <trkseg>
"#,
    );

    for point in points {
        text.push_str(&format!(
            r#"      <trkpt lat="{}" lon="{}" />"#,
            format_float(point.lat),
            format_float(point.lon)
        ));
        text.push('\n');
    }

    text.push_str("    </trkseg>\n  </trk>\n</gpx>\n");
    std::fs::write(output_path, text).map_err(|err| err.to_string())
}

fn write_points_kml(output_path: &Path, points: &[TrackPoint]) -> Result<(), String> {
    let coordinates = points
        .iter()
        .map(|point| format!("{},{},0", format_float(point.lon), format_float(point.lat)))
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2">
  <Document>
    <name>converted</name>
    <Placemark>
      <LineString>
        <coordinates>{coordinates}</coordinates>
      </LineString>
    </Placemark>
  </Document>
</kml>
"#
    );
    std::fs::write(output_path, text).map_err(|err| err.to_string())
}

fn transform_gpx_element(
    elem: &mut Element,
    transform: fn(f64, f64) -> (f64, f64),
) -> Result<usize, String> {
    let mut count = 0;
    if matches!(local_name(&elem.name).as_str(), "wpt" | "trkpt" | "rtept") {
        let lat = elem.attributes.get("lat").cloned();
        let lon = elem.attributes.get("lon").cloned();
        if let (Some(lat), Some(lon)) = (lat, lon) {
            let lat = parse_float(&lat, "GPX 纬度")?;
            let lon = parse_float(&lon, "GPX 经度")?;
            let (new_lon, new_lat) = transform(lon, lat);
            elem.attributes
                .insert("lon".to_string(), format_float(new_lon));
            elem.attributes
                .insert("lat".to_string(), format_float(new_lat));
            count += 1;
        }
    }

    for child in &mut elem.children {
        if let XMLNode::Element(child) = child {
            count += transform_gpx_element(child, transform)?;
        }
    }

    Ok(count)
}

fn transform_kml_element(
    elem: &mut Element,
    transform: fn(f64, f64) -> (f64, f64),
) -> Result<usize, String> {
    let mut count = 0;
    if local_name(&elem.name) == "coordinates" {
        for child in &mut elem.children {
            if let XMLNode::Text(text) = child {
                let (converted, converted_count) = convert_kml_coordinates(text, transform)?;
                *text = converted;
                count += converted_count;
            }
        }
    }

    for child in &mut elem.children {
        if let XMLNode::Element(child) = child {
            count += transform_kml_element(child, transform)?;
        }
    }

    Ok(count)
}

fn collect_gpx_points(
    elem: &Element,
    transform: fn(f64, f64) -> (f64, f64),
    points: &mut Vec<TrackPoint>,
) -> Result<(), String> {
    if matches!(local_name(&elem.name).as_str(), "wpt" | "trkpt" | "rtept") {
        if let (Some(lat), Some(lon)) = (elem.attributes.get("lat"), elem.attributes.get("lon")) {
            let lat = parse_float(lat, "GPX 纬度")?;
            let lon = parse_float(lon, "GPX 经度")?;
            let (new_lon, new_lat) = transform(lon, lat);
            points.push(TrackPoint {
                lon: new_lon,
                lat: new_lat,
                name: None,
            });
        }
    }

    for child in &elem.children {
        if let XMLNode::Element(child) = child {
            collect_gpx_points(child, transform, points)?;
        }
    }

    Ok(())
}

fn collect_kml_points(
    elem: &Element,
    transform: fn(f64, f64) -> (f64, f64),
    points: &mut Vec<TrackPoint>,
) -> Result<(), String> {
    if local_name(&elem.name) == "coordinates" {
        for child in &elem.children {
            if let XMLNode::Text(text) = child {
                for token in text.split_whitespace() {
                    let fields = token.split(',').collect::<Vec<_>>();
                    if fields.len() < 2 {
                        continue;
                    }
                    let lon = parse_float(fields[0], "KML 经度")?;
                    let lat = parse_float(fields[1], "KML 纬度")?;
                    let (new_lon, new_lat) = transform(lon, lat);
                    points.push(TrackPoint {
                        lon: new_lon,
                        lat: new_lat,
                        name: None,
                    });
                }
            }
        }
    }

    for child in &elem.children {
        if let XMLNode::Element(child) = child {
            collect_kml_points(child, transform, points)?;
        }
    }

    Ok(())
}

fn convert_kml_coordinates(
    text: &str,
    transform: fn(f64, f64) -> (f64, f64),
) -> Result<(String, usize), String> {
    let mut converted = Vec::new();
    let mut count = 0;

    for token in text.split_whitespace() {
        let mut fields = token.split(',').map(ToString::to_string).collect::<Vec<_>>();
        if fields.len() < 2 {
            converted.push(token.to_string());
            continue;
        }

        let lon = parse_float(&fields[0], "KML 经度")?;
        let lat = parse_float(&fields[1], "KML 纬度")?;
        let (new_lon, new_lat) = transform(lon, lat);
        fields[0] = format_float(new_lon);
        fields[1] = format_float(new_lat);
        converted.push(fields.join(","));
        count += 1;
    }

    Ok((converted.join(" "), count))
}

fn read_xml(path: &Path) -> Result<Element, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    Element::parse(file).map_err(|err| err.to_string())
}

fn write_xml(path: &Path, root: &Element) -> Result<(), String> {
    let mut file = File::create(path).map_err(|err| err.to_string())?;
    root.write_with_config(
        &mut file,
        EmitterConfig::new()
            .perform_indent(true)
            .write_document_declaration(true),
    )
    .map_err(|err| err.to_string())
}

fn csv_column_indexes(
    headers: Option<&[String]>,
    request: &BatchRequest,
) -> Result<(usize, usize), String> {
    let lat_index = csv_column_index(
        request.csv_lat_column.as_deref(),
        headers,
        &["lat", "latitude", "y"],
        "纬度列",
    )?;
    let lon_index = csv_column_index(
        request.csv_lon_column.as_deref(),
        headers,
        &["lon", "lng", "long", "longitude", "x"],
        "经度列",
    )?;
    Ok((lat_index, lon_index))
}

fn csv_column_index(
    requested: Option<&str>,
    headers: Option<&[String]>,
    fallback_names: &[&str],
    label: &str,
) -> Result<usize, String> {
    if let Some(requested) = requested.filter(|value| !value.trim().is_empty()) {
        if let Ok(index) = requested.trim().parse::<usize>() {
            return index
                .checked_sub(1)
                .ok_or_else(|| format!("{label} 的序号必须从 1 开始"));
        }

        let headers = headers.ok_or_else(|| format!("无表头 CSV 的 {label} 必须填写列序号"))?;
        let normalized = headers
            .iter()
            .map(|name| normalize_column_name(name))
            .collect::<Vec<_>>();
        let key = normalize_column_name(requested);
        return normalized
            .iter()
            .position(|name| name == &key)
            .ok_or_else(|| format!("找不到 CSV {label}：{requested}"));
    }

    let headers = headers.ok_or_else(|| format!("请填写 CSV {label}"))?;
    let normalized = headers
        .iter()
        .map(|name| normalize_column_name(name))
        .collect::<Vec<_>>();

    fallback_names
        .iter()
        .find_map(|name| normalized.iter().position(|header| header.as_str() == *name))
        .ok_or_else(|| format!("无法自动识别 CSV {label}，请手动填写"))
}

fn normalize_column_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn detect_format(path: &Path, requested: &str) -> Result<String, String> {
    let requested = requested.trim().to_lowercase();
    if requested != "auto" {
        return normalized_input_format(&requested);
    }

    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    normalized_input_format(&ext)
}

fn normalized_input_format(value: &str) -> Result<String, String> {
    match value {
        "csv" | "gpx" | "kml" | "gdb" => Ok(value.to_string()),
        _ => Err(format!("无法识别输入格式：{value}")),
    }
}

fn normalized_output_format(value: &str) -> Result<&str, String> {
    match value.trim().to_lowercase().as_str() {
        "csv" => Ok("csv"),
        "gpx" => Ok("gpx"),
        "kml" => Ok("kml"),
        "gdb" => Err("GDB 输出尚未实现".to_string()),
        value => Err(format!("无法识别输出格式：{value}")),
    }
}

fn make_output_path(input: &Path, output_format: &str, mode: &str) -> Result<PathBuf, String> {
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "无法读取文件名".to_string())?;
    let suffix = match mode {
        "add" => "_gcj02",
        "remove" => "_wgs84",
        "none" => "_converted",
        _ => return Err(format!("未知坐标处理模式：{mode}")),
    };
    Ok(input.with_file_name(format!("{stem}{suffix}.{output_format}")))
}

fn transform_for_mode(mode: &str) -> Result<fn(f64, f64) -> (f64, f64), String> {
    match mode {
        "add" => Ok(wgs84_to_gcj02),
        "remove" => Ok(gcj02_to_wgs84),
        "none" => Ok(identity),
        _ => Err(format!("未知坐标处理模式：{mode}")),
    }
}

fn identity(lon: f64, lat: f64) -> (f64, f64) {
    (lon, lat)
}

fn wgs84_to_gcj02(lon: f64, lat: f64) -> (f64, f64) {
    if out_of_china(lon, lat) {
        return (lon, lat);
    }

    let (dlon, dlat) = delta(lon, lat);
    (lon + dlon, lat + dlat)
}

fn gcj02_to_wgs84(lon: f64, lat: f64) -> (f64, f64) {
    if out_of_china(lon, lat) {
        return (lon, lat);
    }

    let mut wgs_lon = lon;
    let mut wgs_lat = lat;
    for _ in 0..10 {
        let (shifted_lon, shifted_lat) = wgs84_to_gcj02(wgs_lon, wgs_lat);
        let diff_lon = shifted_lon - lon;
        let diff_lat = shifted_lat - lat;
        wgs_lon -= diff_lon;
        wgs_lat -= diff_lat;
        if diff_lon.abs() < 1e-7 && diff_lat.abs() < 1e-7 {
            break;
        }
    }

    (wgs_lon, wgs_lat)
}

fn out_of_china(lon: f64, lat: f64) -> bool {
    !(MIN_LON..=MAX_LON).contains(&lon) || !(MIN_LAT..=MAX_LAT).contains(&lat)
}

fn delta(lon: f64, lat: f64) -> (f64, f64) {
    let x = lon - 105.0;
    let y = lat - 35.0;
    let mut dlat = transform_lat(x, y);
    let mut dlon = transform_lon(x, y);
    let radlat = lat / 180.0 * std::f64::consts::PI;
    let sin_radlat = radlat.sin();
    let magic = 1.0 - EE * sin_radlat * sin_radlat;
    let sqrt_magic = magic.sqrt();

    dlat = (dlat * 180.0) / (((A * (1.0 - EE)) / (magic * sqrt_magic)) * std::f64::consts::PI);
    dlon = (dlon * 180.0) / ((A / sqrt_magic) * radlat.cos() * std::f64::consts::PI);
    (dlon, dlat)
}

fn transform_lat(x: f64, y: f64) -> f64 {
    let mut ret = -100.0 + 2.0 * x + 3.0 * y + 0.2 * y * y;
    ret += 0.1 * x * y + 0.2 * x.abs().sqrt();
    ret += (20.0 * (6.0 * x * std::f64::consts::PI).sin()
        + 20.0 * (2.0 * x * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (20.0 * (y * std::f64::consts::PI).sin()
        + 40.0 * (y / 3.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (160.0 * (y / 12.0 * std::f64::consts::PI).sin()
        + 320.0 * (y * std::f64::consts::PI / 30.0).sin())
        * 2.0
        / 3.0;
    ret
}

fn transform_lon(x: f64, y: f64) -> f64 {
    let mut ret = 300.0 + x + 2.0 * y + 0.1 * x * x;
    ret += 0.1 * x * y + 0.1 * x.abs().sqrt();
    ret += (20.0 * (6.0 * x * std::f64::consts::PI).sin()
        + 20.0 * (2.0 * x * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (20.0 * (x * std::f64::consts::PI).sin()
        + 40.0 * (x / 3.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret += (150.0 * (x / 12.0 * std::f64::consts::PI).sin()
        + 300.0 * (x / 30.0 * std::f64::consts::PI).sin())
        * 2.0
        / 3.0;
    ret
}

fn parse_float(value: &str, label: &str) -> Result<f64, String> {
    value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label} 不是有效数字：{value}"))
}

fn format_float(value: f64) -> String {
    let mut text = format!("{value:.8}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        return "0".to_string();
    }
    text
}

fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![convert_files])
        .run(tauri::generate_context!())
        .expect("error while running Track Converter");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_china_is_unchanged() {
        let lon = -122.4194;
        let lat = 37.7749;
        assert_eq!(wgs84_to_gcj02(lon, lat), (lon, lat));
        assert_eq!(gcj02_to_wgs84(lon, lat), (lon, lat));
    }

    #[test]
    fn add_offset_known_beijing_point() {
        let (lon, lat) = wgs84_to_gcj02(116.397128, 39.916527);
        assert!((lon - 116.40337249).abs() < 0.000001);
        assert!((lat - 39.91793075).abs() < 0.000001);
    }

    #[test]
    fn round_trip_is_close() {
        let samples = [
            (116.397128, 39.916527),
            (121.473701, 31.230416),
            (113.264385, 23.129112),
            (114.057868, 22.543099),
        ];

        for (lon, lat) in samples {
            let (shifted_lon, shifted_lat) = wgs84_to_gcj02(lon, lat);
            let (restored_lon, restored_lat) = gcj02_to_wgs84(shifted_lon, shifted_lat);
            assert!(((restored_lon - lon).powi(2) + (restored_lat - lat).powi(2)).sqrt() < 0.000001);
        }
    }
}
