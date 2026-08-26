use std::path::PathBuf;

pub fn run_raw_bridge(socket_path: String, service_name: String) -> Result<i32, String> {
    let sock = PathBuf::from(socket_path);
    if !sock.exists() {
        return Err("Session daemon socket not found. Is the workspace launched?".to_string());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed building Tokio runtime: {}", e))?;

    rt.block_on(async move {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
        use std::io::{Read, Write};

        let stream = tokio::net::UnixStream::connect(&sock)
            .await
            .map_err(|e| format!("Failed connecting to daemon socket: {}", e))?;

        let (reader, mut writer) = stream.into_split();
        let mut buf_reader = BufReader::new(reader);

        // Send AttachRaw request
        let req = serde_json::to_string(&crate::daemon::DaemonRequest::AttachRaw {
            service: service_name.clone(),
        }).map_err(|e| e.to_string())? + "\n";

        writer.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
        writer.flush().await.map_err(|e| e.to_string())?;

        // Read confirmation response line
        let mut resp_line = String::new();
        buf_reader.read_line(&mut resp_line).await.map_err(|e| e.to_string())?;

        let resp: crate::daemon::DaemonResponse = serde_json::from_str(resp_line.trim())
            .map_err(|e| format!("Invalid daemon response: {}", e))?;

        match resp {
            crate::daemon::DaemonResponse::Success => {},
            crate::daemon::DaemonResponse::Error { message } => {
                return Err(format!("Daemon error: {}", message));
            }
            _ => return Err("Unexpected daemon response".to_string()),
        }

        // Enable raw terminal mode
        let _ = enable_raw_mode();

        // Spawn stdin reader task
        let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(128);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 1024];
            while let Ok(n) = stdin.read(&mut buf) {
                if n == 0 {
                    break;
                }
                // Check detach key: Ctrl+] (0x1D), Ctrl+Q (0x11), or Ctrl+W (0x17)
                if n == 1 && (buf[0] == 0x1D || buf[0] == 0x11 || buf[0] == 0x17) {
                    break;
                }

                if stdin_tx.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });

        // Run bidirectional streaming
        let mut stdout = std::io::stdout();
        let mut sock_buf = [0u8; 4096];

        loop {
            tokio::select! {
                res = buf_reader.read(&mut sock_buf) => {
                    match res {
                        Ok(0) => break,
                        Ok(n) => {
                            let _ = stdout.write_all(&sock_buf[..n]);
                            let _ = stdout.flush();
                        }
                        Err(_) => break,
                    }
                }
                Some(data) = stdin_rx.recv() => {
                    if writer.write_all(&data).await.is_err() {
                        break;
                    }
                    let _ = writer.flush().await;
                }
            }
        }

        let _ = disable_raw_mode();
        Ok(0)
    })
}
