use super::*;
fn page(document: &Document, width: usize, height: usize, scroll: usize) -> Viewport {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(page) = document.viewport(width, height, scroll, false).unwrap() {
            return page;
        }
        assert!(std::time::Instant::now() < deadline, "layout timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn large_text_renders_only_the_viewport_and_retains_every_byte_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut builder = Builder::new(dir.path()).unwrap();
    let chunk = "👩‍💻e\u{301} request\n".repeat(2000);
    builder
        .append("Header\n", |sink| {
            for _ in 0..64 {
                sink(&chunk)?;
            }
            Ok(())
        })
        .unwrap();
    let document = builder.finish().unwrap();
    let cache = document.files.0.clone();
    let text = std::fs::metadata(cache.join("text")).unwrap().len();
    assert!(text > 1_000_000);
    let first = page(&document, 40, 5, 0);
    assert_eq!(first.lines.len(), 5);
    assert_eq!(first.lines[0].1, "Header");
    assert!(first.lines[1].1.contains("👩‍💻e\u{301} request"));
    let last = page(&document, 40, 5, usize::MAX);
    assert_eq!(last.lines.len(), 5);
    assert!(last.scroll > 100_000);
}

#[test]
fn search_reads_disk_backed_blocks_across_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let mut builder = Builder::new(dir.path()).unwrap();
    builder
        .append("", |sink| {
            sink(&"x".repeat(CHUNK - 1))?;
            sink("Ą👩‍💻needle\n")
        })
        .unwrap();
    builder.append("", |sink| sink("unrelated record")).unwrap();
    let document = builder.finish().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let page = loop {
        if let Some(page) = document
            .viewport_filtered(80, 4, usize::MAX, false, "Ą👩‍💻needle")
            .unwrap()
        {
            break page;
        }
        assert!(std::time::Instant::now() < deadline, "layout timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let text = page
        .lines
        .into_iter()
        .map(|(_, s)| s)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Ą👩‍💻needle"));
    assert!(!text.contains("unrelated"));
}

#[test]
fn replacing_a_view_cancels_background_layouts_and_releases_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut builder = Builder::new(dir.path()).unwrap();
    let chunk = "long line of text\n".repeat(3000);
    builder
        .append("Header\n", |sink| {
            for _ in 0..64 {
                sink(&chunk)?;
            }
            Ok(())
        })
        .unwrap();
    let document = builder.finish().unwrap();
    let path = document.files.0.clone();
    let alive = document.alive.clone();
    for width in 1..20 {
        let _ = document.viewport(width, 10, 0, false).unwrap();
    }
    assert!(document.layouts.lock().unwrap().len() <= 4);
    drop(document);
    assert_eq!(alive.strong_count(), 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!path.exists());
}
