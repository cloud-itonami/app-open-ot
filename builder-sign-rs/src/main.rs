//! builder-sign CLI — open-ot builder signing per SPEC §9.

use anyhow::{bail, Context, Result};
use builder_sign_rs::{
    hash_file, parse_public_key_hex, sign_hash, verify_hash, Keypair, Signature,
};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "builder-sign", version, about = "open-ot builder signing CLI per SPEC §9")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Generate a fresh Ed25519 keypair.
    Keygen {
        /// Output path for the private key (32-byte hex). Mode 0600. If
        /// omitted, prints to stdout instead.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Sign a file — emit BLAKE3 CID + Ed25519 signature.
    Sign {
        /// Path to the file to sign (typically a .wasm or .aot artefact).
        #[arg(long)]
        input: PathBuf,
        /// Path to a hex-encoded Ed25519 private key file. Use `-` for stdin.
        #[arg(long)]
        key: String,
    },
    /// Verify a signature against a file and public key.
    Verify {
        #[arg(long)]
        input: PathBuf,
        /// Hex-encoded Ed25519 signature (64 bytes).
        #[arg(long)]
        signature: String,
        /// Hex-encoded Ed25519 public key (32 bytes).
        #[arg(long)]
        public_key: String,
    },
    /// Print just the BLAKE3 CID for a file (no signing).
    Cid {
        #[arg(long)]
        input: PathBuf,
    },
}

fn read_key_string(spec: &str) -> Result<String> {
    if spec == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        fs::read_to_string(spec).with_context(|| format!("read key file {}", spec))
    }
}

fn cmd_keygen(out: Option<PathBuf>) -> Result<()> {
    let kp = Keypair::generate();
    let secret = kp.secret_hex();
    let public = kp.public_hex();
    if let Some(path) = out {
        fs::write(&path, &secret).context("write private key file")?;
        // Mode 0600 — Unix only; on Windows the chmod is a no-op.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&path, perms)?;
        }
        eprintln!(
            "[builder-sign] wrote private key to {} (32 bytes hex, mode 0600 on Unix)",
            path.display()
        );
        eprintln!("[builder-sign] public key (hex): {}", public);
    } else {
        println!("{}", secret);
        eprintln!("[builder-sign] public key (hex): {}", public);
    }
    Ok(())
}

fn cmd_sign(input: PathBuf, key: String) -> Result<()> {
    if !input.exists() {
        bail!("input file not found: {}", input.display());
    }
    let key_hex = read_key_string(&key)?;
    let kp = Keypair::from_secret_hex(&key_hex)?;
    let cid = hash_file(&input)?;
    let sig = sign_hash(&cid, &kp);
    let size = fs::metadata(&input)?.len();
    println!("cid_blake3:  {}", cid.to_string());
    println!("sig_ed25519: {}", sig.to_hex());
    println!("size_bytes:  {}", size);
    println!("pubkey_hex:  {}", kp.public_hex());
    Ok(())
}

fn cmd_verify(input: PathBuf, signature: String, public_key: String) -> Result<()> {
    if !input.exists() {
        bail!("input file not found: {}", input.display());
    }
    let cid = hash_file(&input)?;
    let sig = Signature::from_hex(&signature)?;
    let pk = parse_public_key_hex(&public_key)?;
    eprintln!("[builder-sign] cid_blake3:  {}", cid.to_string());
    match verify_hash(&cid, &sig, &pk) {
        Ok(()) => {
            eprintln!("[builder-sign] signature:   VALID");
            Ok(())
        }
        Err(e) => {
            eprintln!("[builder-sign] signature:   INVALID");
            bail!("{}", e);
        }
    }
}

fn cmd_cid(input: PathBuf) -> Result<()> {
    if !input.exists() {
        bail!("input file not found: {}", input.display());
    }
    let cid = hash_file(&input)?;
    println!("{}", cid.to_string());
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Keygen { out } => cmd_keygen(out),
        Cmd::Sign { input, key } => cmd_sign(input, key),
        Cmd::Verify {
            input,
            signature,
            public_key,
        } => cmd_verify(input, signature, public_key),
        Cmd::Cid { input } => cmd_cid(input),
    }
}
