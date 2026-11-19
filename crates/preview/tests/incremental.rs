use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

use fast_preview::{Event, RasterRequest, Settings, Snapshot, Worker};

fn snapshot(root: &Path, path: &Path, text: &str) -> Snapshot {
    Snapshot {
        settings: Settings {
            root: root.into(),
            font_paths: Vec::new(),
            system_fonts: false,
            inputs: Default::default(),
        },
        main: path.into(),
        active: path.into(),
        text: text.into(),
        other_buffers: Vec::new(),
    }
}

fn await_event(worker: &Worker, wake: &mpsc::Receiver<()>) -> Event {
    loop {
        if let Some(event) = worker.poll().expect("preview worker connection") {
            return event;
        }
        wake.recv_timeout(Duration::from_secs(10))
            .expect("preview worker deadline");
    }
}

#[test]
#[ignore = "performance benchmark, run with --release"]
fn incremental_twenty_pages() -> Result<(), Box<dyn std::error::Error>> {
    let (signal, wake) = mpsc::channel();
    let worker = Worker::new(move || {
        let _receiver_closed = signal.send(()).is_err();
    })?;
    let root = tempfile::tempdir()?;
    let path: PathBuf = root.path().join("main.typ");
    let mut base = String::from("#set page(paper: \"a4\", margin: 40pt)\n");
    for page in 1..=20 {
        base.push_str(&format!("= Section {page}\n\n"));
        for _ in 0..8 {
            base.push_str("The measured value follows from $x^2 + y^2 = z^2$. Each experiment compares observed results against a fixed reference value.\n\n");
        }
        if page < 20 {
            base.push_str("#pagebreak()\n");
        }
    }
    let mut samples = Vec::new();
    let mut neighbor = None;
    let mut previous = None;
    for generation in 0..21 {
        let text = base.replacen("measured value", &format!("measured value {generation}"), 1);
        let start = Instant::now();
        worker.compile(generation, snapshot(root.path(), &path, &text));
        let Event::Compiled { result, .. } = await_event(&worker, &wake) else {
            panic!("expected compile completion")
        };
        assert_eq!(result?.len(), 20);
        worker.raster(
            generation,
            RasterRequest {
                pages: 0..2,
                scale: 8,
                inverted: false,
            },
        );
        let Event::Rasterized { result, .. } = await_event(&worker, &wake) else {
            panic!("expected visible pages")
        };
        let bitmaps = result?;
        let elapsed = start.elapsed();
        assert_eq!(bitmaps.len(), 2);
        if let Some(last) = previous {
            assert_ne!(
                last, bitmaps[0].rgba,
                "the edited page should change pixels"
            );
        }
        previous = Some(bitmaps[0].rgba.clone());
        if let Some(id) = neighbor {
            assert_eq!(id, bitmaps[1].id, "reuse the unchanged page raster");
        }
        neighbor = Some(bitmaps[1].id);
        if generation > 0 {
            samples.push(elapsed);
        }
    }
    assert!(!path.exists(), "the preview must compile from memory");
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p95 = samples[18];
    println!(
        "20 pages, 20 incremental edits, compile plus two page rasters: median {median:.2?}, p95 {p95:.2?}"
    );
    assert!(
        median < Duration::from_millis(50),
        "typical update took {median:.2?}"
    );
    Ok(())
}
