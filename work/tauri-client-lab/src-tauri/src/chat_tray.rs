use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

pub fn install(app: &AppHandle, window_label: &'static str) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "chatx-open", "Buka ChatX", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "chatx-quit", "Keluar ChatX", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("chatx-tray")
        .tooltip("ChatX")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "chatx-open" => show(app, window_label),
            "chatx-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if matches!(event, TrayIconEvent::Click {
                button: MouseButton::Left, button_state: MouseButtonState::Up, ..
            }) {
                show(tray.app_handle(), window_label);
            }
        });
    tray = tray.icon(window_icon()?);
    tray.build(app)?;
    Ok(())
}

fn show(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Decode the largest PNG frame from the same ICO embedded in the EXE and installer.
pub fn window_icon() -> tauri::Result<tauri::image::Image<'static>> {
    let ico = include_bytes!("../icons/icon.ico");
    let image = ico.windows(8).rposition(|bytes| bytes == b"\x89PNG\r\n\x1a\n")
        .ok_or_else(|| std::io::Error::other("Frame ikon ChatX tidak tersedia"))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&ico[image..]));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(std::io::Error::other)?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or_else(|| std::io::Error::other("Ikon ChatX terlalu besar"))?];
    let frame = reader.next_frame(&mut pixels).map_err(std::io::Error::other)?;
    if frame.color_type != png::ColorType::Rgba {
        return Err(std::io::Error::other("Ikon ChatX harus memakai RGBA").into());
    }
    pixels.truncate(frame.buffer_size());
    Ok(tauri::image::Image::new_owned(pixels, frame.width, frame.height))
}

pub fn download_feedback(webview: tauri::Webview, event: tauri::webview::DownloadEvent<'_>) -> bool {
    use tauri::webview::DownloadEvent;
    let message = match event {
        DownloadEvent::Requested { .. } => "Unduhan dimulai.".to_owned(),
        DownloadEvent::Finished { success: true, path, .. } => path
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .map_or_else(|| "Unduhan selesai.".to_owned(), |name| format!("File tersimpan: {name}")),
        DownloadEvent::Finished { .. } => "Unduhan dibatalkan atau gagal.".to_owned(),
        _ => return true,
    };
    let detail = serde_json::to_string(&message).unwrap_or_default();
    let _ = webview.eval(format!("window.dispatchEvent(new CustomEvent('kc-download-status',{{detail:{detail}}}));"));
    true
}
