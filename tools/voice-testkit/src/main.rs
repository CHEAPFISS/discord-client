//! Локальный голосовой шлюз для ручной проверки транспорта и микрофона.
//!
//! DAVE выключен (`dave_protocol_version = 0`): проверяется oto Voice WS,
//! UDP discovery, RTP и Opus-отправка, но не MLS-шифрование. Сертификат
//! самоподписанный, путь к PEM печатается при старте.

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use oto_testkit::{
    FakeUdpServer, FakeUdpServerConfig, FakeVoiceGateway, FakeVoiceGatewayConfig, ManualClock,
};

fn write_cert_pem(path: &Path, der: &[u8]) -> std::io::Result<()> {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in encoded.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, pem)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let clock = ManualClock::new(Duration::ZERO);
    let mut udp_config = FakeUdpServerConfig::localhost(clock);
    // Запас на всплеск: при переполнении capture UDP-сервер завершается.
    udp_config.capture_capacity = 4_096;
    let udp = FakeUdpServer::start(udp_config).await?;

    let mut config = FakeVoiceGatewayConfig::local();
    config.voice_ip = udp.local_addr().ip().to_string();
    config.voice_port = udp.local_addr().port();
    config.dave_protocol_version = 0;
    // Heartbeat-ы идут каждые 10 секунд; на длинном ручном тесте
    // дефолтных 128 записей мало — шлюз завершится при переполнении.
    config.capture_capacity = 4_096;

    let gateway = FakeVoiceGateway::start(config).await?;

    let cert_path = std::env::var("TESTKIT_CERT_PATH")
        .unwrap_or_else(|_| "/tmp/opencode/oto-testkit-cert.pem".to_string());
    write_cert_pem(Path::new(&cert_path), gateway.tls().certificate().as_ref())?;

    // Порт случайный, сертификат выписан на localhost — используем это имя.
    let endpoint = format!("localhost:{}", gateway.local_addr().port());

    // Capture-буфер тесткита не бесконечный: при переполнении UDP-сервер
    // завершается. Поэтому постоянно вычитываем пакеты и считаем их сами.
    let captured_total = Arc::new(AtomicUsize::new(0));
    let drain_task = {
        let capture = udp.capture();
        let total = Arc::clone(&captured_total);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(100));
            loop {
                ticker.tick().await;
                total.fetch_add(capture.drain().len(), Ordering::Relaxed);
            }
        })
    };

    println!("ENDPOINT={endpoint}");
    println!("UDP={}", udp.local_addr());
    println!("CERT={cert_path}");
    println!("DAVE=off");
    println!("STATS udp_packets=0 discovery_responses=0 gateway_records=0");
    std::io::stdout().flush()?;

    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = ticker.tick() => {
                println!(
                    "STATS udp_packets={} discovery_responses={} gateway_records={}",
                    captured_total.load(Ordering::Relaxed),
                    udp.stats().responses_sent(),
                    gateway.records().len(),
                );
                std::io::stdout().flush()?;
            }
        }
    }

    drain_task.abort();
    udp.shutdown().await?;
    gateway.shutdown().await?;
    Ok(())
}
