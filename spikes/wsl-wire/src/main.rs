//! 検証4のコマンド。
//! - `wsl-wire serve`: WSL 側で動くサーバー。標準入出力で要求を受ける。
//! - `wsl-wire client --distro <d> --bin <linux-path> --root <linux-path>`: Windows 側から計測する。

use std::io;
use std::time::Instant;

use wsl_wire::{Request, Response, serve, spawn_server};

const PINGS: u64 = 1000;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(io::stdin().lock(), io::stdout().lock()),
        Some("client") => client(&args[1..]),
        _ => {
            eprintln!("使い方: wsl-wire serve | wsl-wire client --distro <d> --bin <path> --root <path>");
            std::process::exit(2);
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> &'a str {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or_else(|| panic!("{name} を指定する"))
}

fn client(args: &[String]) -> io::Result<()> {
    let (distro, bin, root) = (flag(args, "--distro"), flag(args, "--bin"), flag(args, "--root"));
    let mut conn = spawn_server(distro, bin)?;

    let mut micros: Vec<u128> = Vec::with_capacity(PINGS as usize);
    for id in 0..PINGS {
        let start = Instant::now();
        let res = conn.call(&Request::Ping { id })?;
        micros.push(start.elapsed().as_micros());
        assert_eq!(res, Response::Pong { id });
    }
    micros.sort_unstable();
    let pct = |p: f64| micros[((micros.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
    println!("ping x{PINGS}: p50 {:.3} ms, p99 {:.3} ms", pct(0.50), pct(0.99));

    let start = Instant::now();
    let res = conn.call(&Request::Walk { id: 1, root: root.to_string() })?;
    println!("walk via server: {res:?}（往復 {} ms）", start.elapsed().as_millis());

    let unc = format!(r"\\wsl.localhost\{distro}{}", root.replace('/', r"\"));
    match wsl_wire::walk_count(std::path::Path::new(&unc)) {
        Ok((count, ms)) => println!("walk via UNC: count {count}, {ms} ms（{unc}）"),
        Err(e) => println!("walk via UNC: {e}"),
    }
    Ok(())
}
