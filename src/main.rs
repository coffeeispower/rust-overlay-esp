pub mod asset;
pub mod github;
pub mod nix;

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};

use clap::Parser;
use color_eyre::Result;
use futures_util::{stream, StreamExt};
use par_stream::prelude::*;
use tracing::{info, Level};
use tracing_subscriber::{prelude::*, EnvFilter};

use crate::nix::targets::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install().unwrap();
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();
    let stdout_layer = tracing_subscriber::fmt::layer().with_filter(env_filter);
    tracing_subscriber::registry().with(stdout_layer).init();

    let args = Args::parse();
    let client = github::new_client()?;
    let targets = [AARCH64_DARWIN, AARCH64_LINUX, X86_64_DARWIN, X86_64_LINUX];

    let rust_release = github::fetch_latest_release(&client, "esp-rs/rust-build").await?;
    let llvm_release = github::fetch_latest_release(&client, "espressif/llvm-project").await?;
    let esp_release = github::fetch_latest_release(&client, "espressif/crosstool-NG").await?;
    let releases = [&rust_release, &llvm_release, &esp_release]
        .iter()
        .map(|release| release.html_url.clone())
        .collect::<Vec<_>>();
    info!("fetched latest releases: {releases:?}");

    let rust_src_assets = asset::filter_rust_src_assets(&rust_release);
    let rust_assets = targets
        .iter()
        .flat_map(|target| asset::filter_rust_assets(&rust_release, target))
        .inspect(|target_asset| info!("found rust asset: `{}`", target_asset.asset.name))
        .collect::<Vec<_>>();
    let llvm_assets = targets
        .iter()
        .flat_map(|target| asset::filter_llvm_assets(&llvm_release, target))
        .inspect(|target_asset| info!("found llvm asset: `{}`", target_asset.asset.name))
        .collect::<Vec<_>>();
    let esp_assets = targets
        .iter()
        .flat_map(|target| asset::filter_esp_assets(&esp_release, target))
        .inspect(|target_asset| info!("found esp asset: `{}`", target_asset.asset.name))
        .collect::<Vec<_>>();

    let results = stream::iter([rust_src_assets, rust_assets, llvm_assets, esp_assets].into_iter())
        .flat_map(stream::iter)
        .par_then(None, move |target_asset| async move {
            info!("starting download of `{}`...", target_asset.asset.name);
            let digest = target_asset.fetch_digest().await;
            info!("downloaded `{}`: {:?}", target_asset.asset.name, digest);
            (digest, target_asset)
        })
        .collect::<Vec<_>>()
        .await;

    let output_path = args.output_path;
    let mut manifest = BufWriter::new(File::create(output_path.as_path())?);

    writeln!(manifest, "rec {{")?;
    for result in results {
        let (digest, target_asset) = result;
        let nix_code = target_asset.to_nix_src(&digest?)?;
        write!(manifest, "{}", textwrap::indent(nix_code.as_str(), "  "))?;
    }
    writeln!(manifest, "}}")?;

    info!("wrote manifest to `{}`", output_path.to_string_lossy());
    Ok(())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Output Nix manifest path.
    #[arg(short, long, default_value = "./manifest.nix")]
    output_path: PathBuf,
}