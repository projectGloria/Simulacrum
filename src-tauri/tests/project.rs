//! Directory reader: project detection on small fixture trees, and writing into a project.

use std::fs;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use simulacrum_lib::pipeline::{self, Settings, Source};
use simulacrum_lib::project::{self, ExportInput, Format, Kind, Profile, ProjectReport};

fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("simulacrum-project-{}-{name}", std::process::id())).join(name);
    let _ = fs::remove_dir_all(d.parent().unwrap());
    fs::create_dir_all(&d).unwrap();
    for (rel, body) in files {
        let p = d.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    d
}

fn scan(root: &Path) -> ProjectReport {
    project::scan(root).unwrap()
}

fn paths(r: &ProjectReport, kind: Kind) -> Vec<String> {
    r.detection(kind).unwrap_or_else(|| panic!("{kind:?} not detected in {r:#?}")).targets.iter().map(|t| t.rel_path.clone()).collect()
}

fn png_side(r: &ProjectReport, id: &str) -> u32 {
    match r.target(id).unwrap_or_else(|| panic!("no target {id}")).format {
        Format::Png { side } => side,
        ref f => panic!("{id} is {f:?}"),
    }
}

fn source() -> Source {
    pipeline::from_rgba(RgbaImage::from_fn(256, 256, |x, y| {
        if (x as i32 - 128).pow(2) + (y as i32 - 128).pow(2) < 90 * 90 {
            Rgba([20, 20, 60, 255])
        } else {
            Rgba([0, 0, 0, 0])
        }
    }))
    .unwrap()
}

fn export(root: &Path, ids: &[&str]) -> project::ExportOutcome {
    let src = source();
    let s = Settings::default().validated().unwrap();
    let input = ExportInput { source: &src, settings: &s, overrides: None, mono: None };
    let backup = root.parent().unwrap().join("backup");
    project::export(root, &ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &input, &backup).unwrap()
}

const MANIFEST_CUSTOM: &str = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android">
  <application android:label="x" android:icon="@mipmap/wall_icon" android:roundIcon="@mipmap/wall_icon_round">
  </application>
</manifest>"#;

#[test]
fn gradle_custom_icon_name() {
    let root = fixture("gradle", &[
        ("settings.gradle.kts", "include(\":app\")"),
        ("app/build.gradle.kts", ""),
        ("app/src/main/AndroidManifest.xml", MANIFEST_CUSTOM),
        ("lib/src/main/AndroidManifest.xml", "<manifest><application></application></manifest>"),
    ]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::Android));
    let t = r.target("app/src/main/res").unwrap();
    assert_eq!(t.format, Format::AndroidRes { base: "wall_icon".into(), round: Some("wall_icon_round".into()) });
    assert!(r.target("lib/src/main/res").is_none(), "library module without an icon is skipped");
    let store = r.target("app/src/main/wall_icon-playstore.png").unwrap();
    assert!(!store.default_on);
}

#[test]
fn flutter_with_ios_contents() {
    let contents = r#"{"images":[{"size":"20x20","idiom":"iphone","filename":"Icon-App-20x20@2x.png","scale":"2x"},
        {"size":"1024x1024","idiom":"ios-marketing","filename":"Icon-App-1024x1024@1x.png","scale":"1x"}],"info":{"version":1}}"#;
    let root = fixture("flutter", &[
        ("pubspec.yaml", "name: ada\ndependencies:\n  flutter:\n    sdk: flutter\n"),
        ("android/settings.gradle", ""),
        ("android/app/src/main/AndroidManifest.xml", r#"<manifest><application android:icon="@mipmap/ic_launcher"></application></manifest>"#),
        ("ios/Runner/Assets.xcassets/AppIcon.appiconset/Contents.json", contents),
        ("ios/Runner.xcodeproj/project.pbxproj", ""),
        ("web/index.html", "<html></html>"),
        ("web/icons/Icon-192.png", ""),
        ("windows/runner/resources/app_icon.ico", ""),
    ]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::Flutter));
    assert!(r.detection(Kind::Android).is_none() && r.detection(Kind::Xcode).is_none());
    let p = paths(&r, Kind::Flutter);
    for want in [
        "android/app/src/main/res",
        "ios/Runner/Assets.xcassets/AppIcon.appiconset",
        "web/icons/Icon-maskable-512.png",
        "windows/runner/resources/app_icon.ico",
    ] {
        assert!(p.contains(&want.to_string()), "{want} missing from {p:?}");
    }
    assert_eq!(r.target("web/icons/Icon-maskable-192.png").unwrap().profile, Profile::Maskable);

    let out = export(&root, &["ios/Runner/Assets.xcassets/AppIcon.appiconset"]);
    assert_eq!(out.written.len(), 2);
    let big = image::open(root.join("ios/Runner/Assets.xcassets/AppIcon.appiconset/Icon-App-1024x1024@1x.png")).unwrap();
    assert_eq!((big.width(), big.color().has_alpha()), (1024, false), "App Store icon must be opaque");
    let small = image::open(root.join("ios/Runner/Assets.xcassets/AppIcon.appiconset/Icon-App-20x20@2x.png")).unwrap();
    assert_eq!(small.width(), 40);
}

#[test]
fn tauri_v2_icon_list() {
    let conf = r#"{"bundle":{"icon":["icons/32x32.png","icons/128x128@2x.png","icons/icon.icns","icons/icon.ico","../../evil.png"]}}"#;
    let root = fixture("tauri", &[
        ("src-tauri/tauri.conf.json", conf),
        ("src-tauri/Cargo.toml", ""),
        ("src-tauri/icons/Square44x44Logo.png", ""),
        ("index.html", "<html></html>"),
        ("package.json", "{}"),
    ]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::Tauri));
    assert_eq!(png_side(&r, "src-tauri/icons/32x32.png"), 32);
    assert_eq!(png_side(&r, "src-tauri/icons/128x128@2x.png"), 256);
    assert_eq!(png_side(&r, "src-tauri/icons/Square44x44Logo.png"), 44);
    assert_eq!(r.target("src-tauri/icons/icon.icns").unwrap().format, Format::Icns);
    assert!(r.targets().all(|t| !t.rel_path.contains("evil")));
    let web = r.detection(Kind::Web).unwrap();
    assert!(web.targets.iter().all(|t| !t.default_on), "frontend favicons are off by default in a Tauri app");
}

#[test]
fn electron_builder_yaml() {
    let root = fixture("electron", &[
        ("package.json", r#"{"devDependencies":{"electron":"30"}}"#),
        ("electron-builder.yml", "appId: x\ndirectories:\n  buildResources: assets\nwin:\n  icon: icon.ico\nmac:\n  icon: icon.icns\n"),
        ("assets/icon.ico", ""),
    ]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::Electron));
    assert_eq!(paths(&r, Kind::Electron), vec!["assets/icon.ico", "assets/icon.icns"]);
}

#[test]
fn mv3_manifest() {
    let manifest = r#"{"manifest_version":3,"name":"x","icons":{"16":"img/i16.png","128":"img/i128.png"},
        "action":{"default_icon":{"32":"img/i32.png","16":"img/i16.png"}}}"#;
    let root = fixture("mv3", &[("manifest.json", manifest), ("popup.html", "")]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::BrowserExtension));
    assert_eq!(png_side(&r, "img/i128.png"), 128);
    assert_eq!(png_side(&r, "img/i32.png"), 32);
    assert!(r.detection(Kind::Web).is_none());
}

#[test]
fn wxt_public_icons() {
    let root = fixture("wxt", &[("wxt.config.ts", "export default {}"), ("public/icon/48.png", ""), ("package.json", "{}")]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::BrowserExtension));
    assert_eq!(paths(&r, Kind::BrowserExtension), vec!["public/icon/48.png"]);
}

#[test]
fn vscode_non_png_icon() {
    let root = fixture("vscode", &[
        ("package.json", r#"{"name":"t","engines":{"vscode":"^1.80"},"icon":"icon.svg"}"#),
        ("media/x.txt", ""),
    ]);
    let r = scan(&root);
    let d = r.detection(Kind::VsCode).unwrap();
    assert_eq!(d.targets[0].rel_path, "media/icon.png");
    assert!(d.notes.iter().any(|n| n.contains("only accepts PNG")));
}

#[test]
fn rust_build_rs_set_icon() {
    let root = fixture("Captura", &[
        ("Cargo.toml", "[package]\nname='c'"),
        ("build.rs", "fn main(){ let mut r = winres::WindowsResource::new(); r.set_icon(\"assets/Captura.ico\"); r.compile().unwrap(); }"),
    ]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::RustWindows));
    assert_eq!(paths(&r, Kind::RustWindows), vec!["assets/Captura.ico"]);
}

#[test]
fn csproj_with_condition() {
    let root = fixture("dotnet", &[(
        "src/App/App.csproj",
        r#"<Project><PropertyGroup><ApplicationIcon Condition="'$(OS)'=='Windows_NT'">..\..\Assets\app.ico</ApplicationIcon></PropertyGroup></Project>"#,
    )]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::DotNet));
    assert_eq!(paths(&r, Kind::DotNet), vec!["Assets/app.ico"]);
}

#[test]
fn pyinstaller_spec() {
    let root = fixture("pyi", &[("app.spec", "a = Analysis(['main.py'])\nexe = EXE(pyz, name='x', icon=['res/app.ico'])\n")]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::PyInstaller));
    assert_eq!(paths(&r, Kind::PyInstaller), vec!["res/app.ico"]);
}

#[test]
fn web_existing_manifest_is_kept() {
    let manifest = r#"{"name":"x","icons":[{"src":"/pwa-192.png","sizes":"192x192"},{"src":"/pwa-mask.png","sizes":"512x512","purpose":"any maskable"}]}"#;
    let root = fixture("web", &[
        ("index.html", r#"<html><head><link rel="icon" type="image/svg+xml" href="/vite.svg"></head></html>"#),
        ("public/manifest.webmanifest", manifest),
    ]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::Web));
    assert_eq!(png_side(&r, "public/pwa-192.png"), 192);
    assert_eq!(r.target("public/pwa-mask.png").unwrap().profile, Profile::Maskable);
    assert!(r.target("public/site.webmanifest").is_none());
    assert!(r.detection(Kind::Web).unwrap().notes.iter().any(|n| n.contains("SVG")));

    let ids: Vec<String> = r.targets().map(|t| t.id.clone()).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    export(&root, &ids);
    assert_eq!(fs::read_to_string(root.join("public/manifest.webmanifest")).unwrap(), manifest);
    let mask = image::open(root.join("public/pwa-mask.png")).unwrap();
    assert_eq!((mask.width(), mask.color().has_alpha()), (512, false));
    assert!(root.join("public/favicon.ico").is_file());
}

#[test]
fn repo_icon() {
    let root = fixture("Vestigium", &[("Vestigium.ico", ""), ("README.md", "")]);
    let r = scan(&root);
    assert_eq!(r.primary, Some(Kind::RepoIcon));
    assert_eq!(paths(&r, Kind::RepoIcon), vec!["Vestigium.ico"]);
}

#[test]
fn traversal_is_rejected() {
    let manifest = r#"{"manifest_version":3,"icons":{"16":"../../outside.png","48":"/abs/../../x.png","128":"C:/Windows/x.png"}}"#;
    let root = fixture("evil", &[("manifest.json", manifest)]);
    let r = scan(&root);
    assert!(r.targets().next().is_none(), "{r:#?}");
    let src = source();
    let s = Settings::default().validated().unwrap();
    let input = ExportInput { source: &src, settings: &s, overrides: None, mono: None };
    assert!(project::export(&root, &["../../outside.png".into()], &input, &root.join("b")).is_err());
}

#[test]
fn android_export_replaces_webp_and_backs_up() {
    let root = fixture("wallupus", &[
        ("settings.gradle", ""),
        ("app/src/main/AndroidManifest.xml", MANIFEST_CUSTOM),
        ("app/src/main/res/mipmap-hdpi/wall_icon.webp", "old webp"),
        ("app/src/main/res/mipmap-hdpi/wall_icon_foreground.png", "old png"),
        ("app/src/main/res/values/strings.xml", "<resources/>"),
    ]);
    let out = export(&root, &["app/src/main/res"]);
    let res = root.join("app/src/main/res");
    assert!(!res.join("mipmap-hdpi/wall_icon.webp").exists());
    assert!(res.join("mipmap-hdpi/wall_icon.png").is_file());
    assert!(res.join("mipmap-anydpi-v26/wall_icon.xml").is_file());
    assert!(res.join("mipmap-anydpi-v26/wall_icon_round.xml").is_file());
    assert!(fs::read_to_string(res.join("mipmap-anydpi-v26/wall_icon.xml")).unwrap().contains("@mipmap/wall_icon_foreground"));
    assert_eq!(out.removed, vec!["app/src/main/res/mipmap-hdpi/wall_icon.webp"]);
    assert_eq!(out.replaced, 1);
    let backup = PathBuf::from(out.backup_dir.unwrap());
    assert_eq!(fs::read_to_string(backup.join("app/src/main/res/mipmap-hdpi/wall_icon.webp")).unwrap(), "old webp");
    assert_eq!(fs::read_to_string(backup.join("app/src/main/res/mipmap-hdpi/wall_icon_foreground.png")).unwrap(), "old png");
    assert_eq!(fs::read_to_string(res.join("values/strings.xml")).unwrap(), "<resources/>");
}
