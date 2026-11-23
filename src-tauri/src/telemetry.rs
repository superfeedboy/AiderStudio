use crate::models::HardwareTelemetry;
use regex::Regex;
use tokio::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "macos")]
static CORES: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
#[cfg(target_os = "macos")]
static MEM_TOTAL_BYTES: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

async fn run_cmd_output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.kill_on_drop(true);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = tokio::time::timeout(
        std::time::Duration::from_millis(800),
        cmd.output(),
    )
    .await
    .ok()?
    .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

pub async fn collect_hardware_telemetry() -> HardwareTelemetry {
    #[cfg(target_os = "macos")]
    {
        collect_macos().await
    }
    #[cfg(target_os = "windows")]
    {
        collect_windows().await
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        HardwareTelemetry {
            cpu_usage: 0.0,
            gpu_usage: None,
            mem_used_gb: 0.0,
            mem_total_gb: 0.0,
            mem_percent: 0.0,
            temp_celsius: None,
            platform: std::env::consts::OS.to_string(),
        }
    }
}

#[cfg(target_os = "macos")]
async fn collect_macos() -> HardwareTelemetry {
    let cores = *CORES.get_or_init(|| {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.logicalcpu"])
            .output()
            .ok()
            .and_then(|s| String::from_utf8(s.stdout).ok())
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(8.0)
            .max(1.0)
    });

    // Use fast sysctl `vm.loadavg` (instantaneous, zero overhead, no child leaks)
    let mut cpu_usage = 0.0_f32;
    if let Some(load_out) = run_cmd_output("sysctl", &["-n", "vm.loadavg"]).await {
        let first = load_out
            .trim()
            .trim_matches(|c| c == '{' || c == '}')
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0);
        cpu_usage = ((first / cores) * 100.0).clamp(0.0, 100.0);
    }

    // Memory via cached `sysctl hw.memsize` & `vm_stat`
    let mem_total_bytes = *MEM_TOTAL_BYTES.get_or_init(|| {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|s| String::from_utf8(s.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(16 * 1024 * 1024 * 1024)
    });
    let mem_total_gb = (mem_total_bytes as f32) / 1_073_741_824.0;

    let mut mem_used_gb = 0.0_f32;
    if let Some(vm) = run_cmd_output("vm_stat", &[]).await {
        let page_size = Regex::new(r"page size of (\d+) bytes")
            .ok()
            .and_then(|re| re.captures(&vm))
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .unwrap_or(16384);

        let extract_pages = |label: &str| -> u64 {
            vm.lines()
                .find(|l| l.starts_with(label))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().trim_end_matches('.').parse::<u64>().ok())
                .unwrap_or(0)
        };

        let active = extract_pages("Pages active");
        let wired = extract_pages("Pages wired down");
        let compressed = extract_pages("Pages occupied by compressor");
        let used_bytes = (active + wired + compressed) * page_size;
        mem_used_gb = (used_bytes as f32) / 1_073_741_824.0;
    }

    let mem_percent = if mem_total_gb > 0.0 {
        ((mem_used_gb / mem_total_gb) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    // Apple Silicon / macOS GPU utilization via ioreg IOAccelerator
    let gpu_usage = if let Some(io_out) =
        run_cmd_output("ioreg", &["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"]).await
    {
        Regex::new(r#""Device Utilization %"\s*=\s*(\d+)"#)
            .ok()
            .and_then(|re| re.captures(&io_out))
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<f32>().ok())
    } else {
        None
    };

    // Temperature via AppleSmartBattery sensor (returns centi-Kelvin or decicelsius * 100) when exposed
    let temp_celsius = if let Some(bat_out) =
        run_cmd_output("ioreg", &["-r", "-d", "1", "-w", "0", "-c", "AppleSmartBattery"]).await
    {
        Regex::new(r#""Temperature"\s*=\s*(\d+)"#)
            .ok()
            .and_then(|re| re.captures(&bat_out))
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<f32>().ok())
            .map(|raw| {
                // Usually decikelvin (e.g. 3045 -> 31.35 °C) or centidegrees C (e.g. 3120 -> 31.2 °C)
                if raw > 2000.0 {
                    if raw > 25000.0 {
                        raw / 100.0 - 273.15
                    } else if raw > 2000.0 && raw < 4000.0 {
                        (raw / 10.0) - 273.15
                    } else {
                        raw / 100.0
                    }
                } else {
                    raw / 10.0
                }
            })
            .filter(|t| *t > 10.0 && *t < 115.0)
    } else {
        None
    };

    HardwareTelemetry {
        cpu_usage: (cpu_usage * 10.0).round() / 10.0,
        gpu_usage,
        mem_used_gb: (mem_used_gb * 10.0).round() / 10.0,
        mem_total_gb: (mem_total_gb * 10.0).round() / 10.0,
        mem_percent: (mem_percent * 10.0).round() / 10.0,
        temp_celsius: temp_celsius.map(|t| (t * 10.0).round() / 10.0),
        platform: "macOS".to_string(),
    }
}

#[cfg(target_os = "windows")]
async fn collect_windows() -> HardwareTelemetry {
    let ps_script = r#"
$cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
$os = Get-CimInstance Win32_OperatingSystem
$totalKb = [double]$os.TotalVisibleMemorySize
$freeKb = [double]$os.FreePhysicalMemory
$usedGb = ($totalKb - $freeKb) / 1048576
$totalGb = $totalKb / 1048576
Write-Output "$cpu|$usedGb|$totalGb"
"#;

    let mut cpu_usage = 0.0_f32;
    let mut mem_used_gb = 0.0_f32;
    let mut mem_total_gb = 0.0_f32;

    if let Some(out) = run_cmd_output(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", ps_script],
    )
    .await
    {
        let parts: Vec<&str> = out.trim().split('|').collect();
        if parts.len() == 3 {
            cpu_usage = parts[0].trim().replace(',', ".").parse::<f32>().unwrap_or(0.0);
            mem_used_gb = parts[1].trim().replace(',', ".").parse::<f32>().unwrap_or(0.0);
            mem_total_gb = parts[2].trim().replace(',', ".").parse::<f32>().unwrap_or(0.0);
        }
    }

    let mem_percent = if mem_total_gb > 0.0 {
        ((mem_used_gb / mem_total_gb) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let mut gpu_usage = None;
    let mut temp_celsius = None;
    if let Some(nv_out) = run_cmd_output(
        "nvidia-smi",
        &[
            "--query-gpu=utilization.gpu,temperature.gpu",
            "--format=csv,noheader,nounits",
        ],
    )
    .await
    {
        if let Some(line) = nv_out.lines().next() {
            let cols: Vec<&str> = line.split(',').collect();
            if cols.len() >= 2 {
                gpu_usage = cols[0].trim().parse::<f32>().ok();
                temp_celsius = cols[1].trim().parse::<f32>().ok();
            }
        }
    }

    HardwareTelemetry {
        cpu_usage: (cpu_usage * 10.0).round() / 10.0,
        gpu_usage,
        mem_used_gb: (mem_used_gb * 10.0).round() / 10.0,
        mem_total_gb: (mem_total_gb * 10.0).round() / 10.0,
        mem_percent: (mem_percent * 10.0).round() / 10.0,
        temp_celsius,
        platform: "Windows".to_string(),
    }
}
