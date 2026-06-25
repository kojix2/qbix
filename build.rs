use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const LIBBIOSYNTAX_VERSION: &str = "v0.1.0";
const LIBBIOSYNTAX_TARBALL_URL: &str =
    "https://github.com/kojix2/libbiosyntax/archive/refs/tags/v0.1.0.tar.gz";

fn main() {
    println!("cargo:rerun-if-env-changed=HTSDIR");
    println!("cargo:rerun-if-env-changed=LIBDEFLATE_PREFIX");
    println!("cargo:rerun-if-env-changed=HTSLIB_STATIC");
    println!("cargo:rerun-if-env-changed=HTS_STATIC");
    println!("cargo:rerun-if-env-changed=LIBBIOSYNTAX_DIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR");

    let htsdir = env::var("HTSDIR").ok().filter(|value| !value.is_empty());
    let libdeflate_prefix = env::var("LIBDEFLATE_PREFIX")
        .ok()
        .filter(|value| !value.is_empty());
    let static_htslib = env_flag("HTSLIB_STATIC")
        || env_flag("HTS_STATIC")
        || htsdir.as_deref().is_some_and(has_static_htslib);
    let pkg_config = pkg_config_htslib(static_htslib);

    build_hts_shim(htsdir.as_deref(), pkg_config.as_ref());
    if env::var_os("CARGO_FEATURE_BIOSYNTAX").is_some() {
        build_biosyntax();
    }

    if let Some(htsdir) = &htsdir {
        println!("cargo:rustc-link-search=native={htsdir}");
        println!("cargo:rustc-link-search=native={htsdir}/lib");
    }
    if let Some(prefix) = &libdeflate_prefix {
        println!("cargo:rustc-link-search=native={prefix}/lib");
    } else {
        emit_pkg_config_libdir("libdeflate");
    }
    if static_htslib && target_os() == "linux" {
        emit_pkg_config_libdir("zlib");
    }
    println!("cargo:rustc-link-lib=static=qbix_hts_shim");
    if env::var_os("CARGO_FEATURE_BIOSYNTAX").is_some() {
        println!("cargo:rustc-link-lib=static=qbix_biosyntax");
    }
    if let Some(pkg_config) = pkg_config {
        emit_pkg_config_libs(&pkg_config.libs, static_htslib);
    } else if static_htslib {
        println!("cargo:rustc-link-lib=static=hts");
    } else {
        println!("cargo:rustc-link-lib=hts");
    }
}

fn build_biosyntax() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let source_dir = libbiosyntax_source_dir(&out_dir);
    let source = source_dir.join("src").join("biosyntax.c");
    let include_dir = source_dir.join("include");
    let object = out_dir.join("biosyntax.o");
    let library = out_dir.join("libqbix_biosyntax.a");

    let mut cc = Command::new(env::var("CC").unwrap_or_else(|_| "cc".to_string()));
    cc.args(["-O2", "-fPIC", "-DBIOSYN_STATIC", "-c"])
        .arg(&source)
        .arg("-I")
        .arg(&include_dir)
        .arg("-o")
        .arg(&object);
    assert!(cc.status().expect("failed to run C compiler").success());

    let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());
    assert!(Command::new(ar)
        .arg("crs")
        .arg(&library)
        .arg(&object)
        .status()
        .expect("failed to run ar")
        .success());

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rerun-if-env-changed=LIBBIOSYNTAX_DIR");
}

fn libbiosyntax_source_dir(out_dir: &Path) -> PathBuf {
    if let Some(path) = env::var("LIBBIOSYNTAX_DIR")
        .ok()
        .filter(|value| !value.is_empty())
    {
        return PathBuf::from(path);
    }

    let source_dir = out_dir.join(format!("libbiosyntax-{LIBBIOSYNTAX_VERSION}"));
    let source_file = source_dir.join("src").join("biosyntax.c");
    if source_file.exists() {
        return source_dir;
    }

    std::fs::create_dir_all(&source_dir).expect("could not create libbiosyntax source directory");
    let tarball = out_dir.join(format!("libbiosyntax-{LIBBIOSYNTAX_VERSION}.tar.gz"));
    assert!(
        Command::new("curl")
            .args(["-L", "--fail", LIBBIOSYNTAX_TARBALL_URL, "-o"])
            .arg(&tarball)
            .status()
            .expect("failed to run curl")
            .success(),
        "could not download libbiosyntax {LIBBIOSYNTAX_VERSION}"
    );
    assert!(
        Command::new("tar")
            .args(["-xzf"])
            .arg(&tarball)
            .args(["--strip-components=1", "-C"])
            .arg(&source_dir)
            .status()
            .expect("failed to run tar")
            .success(),
        "could not unpack libbiosyntax {LIBBIOSYNTAX_VERSION}"
    );
    source_dir
}

fn build_hts_shim(htsdir: Option<&str>, pkg_config: Option<&PkgConfigOutput>) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let object = out_dir.join("hts_shim.o");
    let library = out_dir.join("libqbix_hts_shim.a");

    let mut cc = Command::new(env::var("CC").unwrap_or_else(|_| "cc".to_string()));
    cc.args(["-O2", "-fPIC", "-c", "src/hts_shim.c", "-o"])
        .arg(&object);
    if let Some(htsdir) = htsdir {
        cc.arg(format!("-I{htsdir}"));
        cc.arg(format!("-I{htsdir}/include"));
    }
    if let Some(pkg_config) = pkg_config {
        cc.args(split_flags(&pkg_config.cflags));
    }
    assert!(cc.status().expect("failed to run C compiler").success());

    let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());
    assert!(Command::new(ar)
        .arg("crs")
        .arg(&library)
        .arg(&object)
        .status()
        .expect("failed to run ar")
        .success());

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rerun-if-changed=src/hts_shim.c");
}

struct PkgConfigOutput {
    cflags: String,
    libs: String,
}

fn pkg_config_htslib(static_htslib: bool) -> Option<PkgConfigOutput> {
    let cflags = Command::new("pkg-config")
        .args(["--cflags", "htslib"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_default();

    let mut libs_args = vec!["--libs"];
    if static_htslib {
        libs_args.push("--static");
    }
    libs_args.push("htslib");
    let libs = Command::new("pkg-config")
        .args(libs_args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())?;

    Some(PkgConfigOutput { cflags, libs })
}

fn emit_pkg_config_libs(libs: &str, static_htslib: bool) {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mut emitted_libs = Vec::new();
    for flag in split_flags(libs) {
        if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            emitted_libs.push(lib);
            if static_htslib && should_link_static_lib(lib, &target_os) {
                println!("cargo:rustc-link-lib=static={lib}");
            } else {
                println!("cargo:rustc-link-lib={lib}");
            }
        } else if let Some(arg) = flag.strip_prefix("-Wl,") {
            for part in arg.split(',').filter(|part| !part.is_empty()) {
                println!("cargo:rustc-link-arg={part}");
            }
        } else {
            println!("cargo:rustc-link-arg={flag}");
        }
    }
    if static_htslib {
        emit_static_htslib_fallback_libs(&emitted_libs, &target_os);
    }
}

fn should_link_static_lib(lib: &str, target_os: &str) -> bool {
    if lib == "hts" {
        return true;
    }
    if lib == "deflate" {
        return true;
    }
    target_os == "linux" && lib == "z"
}

fn has_static_htslib(htsdir: &str) -> bool {
    PathBuf::from(htsdir).join("lib").join("libhts.a").exists()
}

fn emit_static_htslib_fallback_libs(emitted_libs: &[&str], target_os: &str) {
    if !emitted_libs.contains(&"deflate") {
        println!("cargo:rustc-link-lib=static=deflate");
    }
    if !emitted_libs.contains(&"z") {
        if target_os == "linux" {
            println!("cargo:rustc-link-lib=static=z");
        } else {
            println!("cargo:rustc-link-lib=z");
        }
    }
}

fn emit_pkg_config_libdir(package: &str) {
    if let Some(libdir) = Command::new("pkg-config")
        .args(["--variable=libdir", package])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        println!("cargo:rustc-link-search=native={libdir}");
    }
}

fn split_flags(flags: &str) -> impl Iterator<Item = &str> {
    flags.split_whitespace().filter(|flag| !flag.is_empty())
}

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn env_flag(name: &str) -> bool {
    matches!(
        env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}
