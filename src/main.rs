use std::{
    env, fs,
    io::Cursor,
    path::PathBuf,
    process,
};

use curl::easy::Easy;
use libc;
use serde::Deserialize;
use tar::Archive;
use xz2::read::XzDecoder;

#[derive(Deserialize)]
struct ReleaseAsset {
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

fn geteuid() -> u32 {
    unsafe { libc::geteuid() }
}

fn download(url: &str, user_agent: Option<&str>) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    let mut easy = Easy::new();
    easy.url(url)?;
    if let Some(ua) = user_agent {
        easy.useragent(ua)?;
    }
    easy.follow_location(true)?;
    {
        let mut transfer = easy.transfer();
        transfer.write_function(|new_data| {
            data.extend_from_slice(new_data);
            Ok(new_data.len())
        })?;
        transfer.perform()?;
    }
    Ok(data)
}

fn detect_arch() -> &'static str {
    if let Ok(flags) = fs::read_to_string("/proc/cpuinfo") {
        if flags.lines()
            .find(|l| l.starts_with("flags"))
            .map_or(false, |f| {
                f.contains("avx2") && f.contains("bmi1") && f.contains("bmi2") && f.contains("fma")
            })
        {
            return "x86_64_v3";
        }
    }
    "x86_64"
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if geteuid() == 0 {
        eprintln!("[-] Do not run as root.");
        process::exit(1);
    }

    let home = env::var("HOME")?;
    let paths = [
        format!("{}/.steam/root/compatibilitytools.d", home),
        format!("{}/.local/share/Steam/compatibilitytools.d", home),
    ];

    let install_dir = paths.iter()
        .find(|p| PathBuf::from(p).exists())
        .unwrap_or(&paths[0]);

    fs::create_dir_all(install_dir)?;

    let arch = detect_arch();

    let (_tag, url) = {
        let api_url = "https://api.github.com/repos/CachyOS/proton-cachyos/releases/latest";
        let json_bytes = download(api_url, Some("protonup-cachyos"))?;
        let release: Release = serde_json::from_slice(&json_bytes)?;
        let asset_url = release.assets.iter()
            .find(|a| a.browser_download_url.ends_with(&format!("{arch}.tar.xz")))
            .ok_or("No matching asset found")?
            .browser_download_url.clone();
        (release.tag_name, asset_url)
    };

    let install_name = url.split('/').last().unwrap().strip_suffix(".tar.xz").unwrap();
    let install_path = PathBuf::from(install_dir).join(install_name);

    if install_path.exists() {
        println!("[✓] Already installed: {}", install_name);
        return Ok(());
    }

    if arch == "x86_64_v3" {
        println!("[*] CPU supports x86_64_v3 — using optimized build");
    } else {
        println!("[*] CPU does not support x86_64_v3 — using baseline x86_64 build");
    }

    println!("[↓] Downloading: {}", url.split('/').last().unwrap());
    let data = download(&url, None)?;

    println!("[>] Extracting...");
    let tmp_dir = env::temp_dir().join("proton_extract");
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)?;
    let tar = XzDecoder::new(Cursor::new(data));
    let mut archive = Archive::new(tar);
    archive.unpack(&tmp_dir)?;

    let extracted_dir = fs::read_dir(&tmp_dir)?
        .find_map(|e| {
            let p = e.ok()?.path();
            (p.is_dir() && p.file_name()?.to_str()?.starts_with("proton-")).then_some(p)
        })
        .ok_or("Extracted folder not found")?;

    fs::rename(&extracted_dir, &install_path)?;

    for entry in fs::read_dir(install_dir)? {
        let p = entry?.path();
        if p != install_path && p.is_dir() {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("proton-cachyos-") {
                    let _ = fs::remove_dir_all(p);
                }
            }
        }
    }

    println!("[✓] Installed: {}", install_name);
    println!("[✓] Done. Restart Steam to use the new version.");

    Ok(())
}

