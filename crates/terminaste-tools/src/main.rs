use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use serde_json::json;
use terminaste_settings::settings_schema_json;

const ICON_SIZES: [u32; 5] = [16, 32, 128, 256, 512];

fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("schema") => write_settings_schema(Path::new("assets/settings.schema.json"))?,
        Some("icon") | None => generate_icons(Path::new("assets"))?,
        Some("licenses") => write_licenses(args.get(1).map(PathBuf::from))?,
        Some("assets") => {
            write_settings_schema(Path::new("assets/settings.schema.json"))?;
            generate_icons(Path::new("assets"))?;
            write_licenses(None)?;
        }
        Some(other) => anyhow::bail!("unknown tool command: {other}"),
    }
    Ok(())
}

fn write_settings_schema(path: &Path) -> anyhow::Result<()> {
    let schema = settings_schema_json()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, schema)?;
    Ok(())
}

fn generate_icons(asset_dir: &Path) -> anyhow::Result<()> {
    let source = image::open(asset_dir.join("icon.png"))?;
    let icon_dir = asset_dir.join("icons");
    std::fs::create_dir_all(&icon_dir)?;
    for size in ICON_SIZES {
        source
            .resize_exact(size, size, FilterType::Lanczos3)
            .save(icon_dir.join(format!("icon-{size}.png")))?;
    }
    Ok(())
}

fn write_licenses(out_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let out_dir = out_dir.unwrap_or_else(|| PathBuf::from("assets"));
    let packages = cargo_lock_packages(Path::new("Cargo.lock"))?;
    let license_text = third_party_license_text(&packages);
    std::fs::create_dir_all(&out_dir)?;
    std::fs::write("THIRD-PARTY-LICENSES.md", &license_text)?;
    std::fs::write(out_dir.join("THIRD-PARTY-LICENSES.md"), license_text)?;
    std::fs::write(
        out_dir.join("dependency-manifest.json"),
        serde_json::to_string_pretty(&dependency_manifest(&packages))?,
    )?;
    Ok(())
}

fn cargo_lock_packages(path: &Path) -> anyhow::Result<Vec<LockedPackage>> {
    let lock = std::fs::read_to_string(path)?;
    let value = lock.parse::<toml::Value>()?;
    let package_values = value
        .get("package")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let workspace_names = workspace_package_names()?;
    let mut packages = Vec::new();
    for package in package_values {
        let name = package
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if name.is_empty() || workspace_names.contains(&name) {
            continue;
        }
        packages.push(LockedPackage {
            name,
            version: package
                .get("version")
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            source: package
                .get("source")
                .and_then(toml::Value::as_str)
                .map(ToOwned::to_owned),
            checksum: package
                .get("checksum")
                .and_then(toml::Value::as_str)
                .map(ToOwned::to_owned),
        });
    }
    packages.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.version.cmp(&right.version))
    });
    packages.dedup_by(|left, right| left.name == right.name && left.version == right.version);
    Ok(packages)
}

fn workspace_package_names() -> anyhow::Result<BTreeSet<String>> {
    let workspace = std::fs::read_to_string("Cargo.toml")?.parse::<toml::Value>()?;
    let members = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut names = BTreeSet::new();
    for member in members {
        let Some(member) = member.as_str() else {
            continue;
        };
        let manifest = Path::new(member).join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let value = std::fs::read_to_string(manifest)?.parse::<toml::Value>()?;
        if let Some(name) = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        {
            names.insert(name.to_owned());
        }
    }
    Ok(names)
}

fn third_party_license_text(packages: &[LockedPackage]) -> String {
    let mut text = String::from(
        "# Third-party licenses\n\nThis bundled notice is generated from Cargo.lock. Review crate license metadata before release.\n\n| Package | Version | Source | Checksum |\n| --- | --- | --- | --- |\n",
    );
    for package in packages {
        text.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            package.name,
            package.version,
            package.source.as_deref().unwrap_or("local"),
            package.checksum.as_deref().unwrap_or("")
        ));
    }
    text
}

fn dependency_manifest(packages: &[LockedPackage]) -> serde_json::Value {
    let dependencies = packages
        .iter()
        .map(|package| {
            json!({
                "name": package.name,
                "version": package.version,
                "source": package.source,
                "checksum": package.checksum,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "generated_from": "Cargo.lock",
        "dependencies": dependencies,
    })
}

#[derive(Debug, Clone)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}
