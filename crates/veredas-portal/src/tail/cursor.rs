//! `TailedFile`: the cursor over one open file. Incremental byte reads, streaming decode with
//! encoding detection, newline splitting with a held partial line, and the identity and size
//! checks everything above it builds on.
//!
//! Why this works on Windows: the standard library opens files with full sharing (read, write,
//! delete), so a writer can append to, rename or replace a file held open here, and the handle
//! keeps reading the original file. Rotation handling relies on exactly that.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use memchr::memchr;
use same_file::Handle;

use veredas_core::SourceError;

use crate::encoding::{DetectedEncoding, TextDecoder};
use crate::tail::FileStart;

pub(crate) const READ_CHUNK: usize = 64 * 1024;

/// The outcome of one `read_new` pass.
#[derive(Debug)]
pub(crate) enum ReadStep {
    /// Whatever was appended was read (possibly nothing).
    Progress,
    /// The file held shrank below the offset: an in-place truncation. The caller decides
    /// (usually `handle_truncation`).
    Truncated,
}

/// What the path points at now, relative to the file held open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathStatus {
    Same,
    /// A different file lives at the path now.
    Replaced,
    /// Nothing openable at the path right now (deleted, mid-rotation, or a transient share
    /// error: indistinguishable, and treated alike).
    Missing,
}

pub(crate) struct TailedFile {
    path: PathBuf,
    /// Owns the open file; doubles as its identity (volume and file index).
    handle: Handle,
    /// Bytes of this file consumed. The OS file cursor mirrors it.
    offset: u64,
    decoder: TextDecoder,
    /// Decoded text still waiting for its newline.
    pending: String,
    /// Decode scratch, reused across reads.
    text: String,
    /// Read scratch, reused across reads.
    buf: Vec<u8>,
    /// Set when a tail starts at the end mid-line: swallow decoded text up to and including the
    /// first newline, so a torn fragment is never emitted.
    skip_until_newline: bool,
}

impl TailedFile {
    pub fn open(path: &Path, start: FileStart) -> Result<Self, SourceError> {
        let file = File::open(path).map_err(|e| SourceError::io(path, e))?;
        let handle = Handle::from_file(file).map_err(|e| SourceError::io(path, e))?;
        let mut tf = TailedFile {
            path: path.to_path_buf(),
            handle,
            offset: 0,
            decoder: TextDecoder::new(),
            pending: String::new(),
            text: String::new(),
            buf: vec![0u8; READ_CHUNK],
            skip_until_newline: false,
        };
        if start == FileStart::End {
            tf.seek_to_end()?;
        }
        Ok(tf)
    }

    /// Open positioned at a byte offset known to be a line start (a backfill computed it from
    /// raw line splitting). Mid-file there is no byte-order mark, so the encoding, when known
    /// from the peek, comes along; otherwise detection falls back to the heuristics. The offset
    /// is clamped to the current length: a file shrinking between locate and open shows as a
    /// truncation, never as a replay of pre-start content.
    pub fn open_at(path: &Path, offset: u64, encoding: Option<DetectedEncoding>) -> Result<Self, SourceError> {
        let file = File::open(path).map_err(|e| SourceError::io(path, e))?;
        let handle = Handle::from_file(file).map_err(|e| SourceError::io(path, e))?;
        let len = handle.as_file().metadata().map(|m| m.len()).map_err(|e| SourceError::io(path, e))?;
        let offset = offset.min(len);
        let mut tf = TailedFile {
            path: path.to_path_buf(),
            handle,
            offset,
            decoder: match encoding {
                Some(enc) => TextDecoder::with_encoding(enc),
                None => TextDecoder::new(),
            },
            pending: String::new(),
            text: String::new(),
            buf: vec![0u8; READ_CHUNK],
            skip_until_newline: false,
        };
        if offset > 0 {
            tf.handle.as_file_mut().seek(SeekFrom::Start(offset)).map_err(|e| SourceError::io(path, e))?;
        }
        Ok(tf)
    }

    /// Jump to the current end. If the last byte is not a newline the file ends mid-line, so the
    /// fragment skip is armed. (A byte-level check: right for UTF-8 and CP1252; a UTF-16 LE
    /// newline still ends in 0x0A, and a false "mid-line" verdict only costs skipping until the
    /// next newline, which the end-start semantics allow.)
    fn seek_to_end(&mut self) -> Result<(), SourceError> {
        let len = self.disk_len()?;
        if len == 0 {
            return Ok(());
        }
        let file = self.handle.as_file_mut();
        file.seek(SeekFrom::End(-1)).map_err(|e| SourceError::io(&self.path, e))?;
        let mut last = [0u8; 1];
        let n = file.read(&mut last).map_err(|e| SourceError::io(&self.path, e))?;
        self.offset = len;
        self.skip_until_newline = !(n == 1 && last[0] == b'\n');
        Ok(())
    }

    /// The length of the file held open (by handle: immune to renames).
    pub fn disk_len(&self) -> Result<u64, SourceError> {
        self.handle.as_file().metadata().map(|m| m.len()).map_err(|e| SourceError::io(&self.path, e))
    }

    pub fn identity(&self) -> &Handle {
        &self.handle
    }

    /// Read everything appended since the last call, emitting completed lines. Bounded by the
    /// length observed at entry, so a fast writer cannot pin the reader in here forever.
    pub fn read_new(&mut self, emit: &mut dyn FnMut(String)) -> Result<ReadStep, SourceError> {
        let disk_len = self.disk_len()?;
        if disk_len < self.offset {
            return Ok(ReadStep::Truncated);
        }
        let mut buf = std::mem::take(&mut self.buf);
        let result = loop {
            if self.offset >= disk_len {
                break Ok(ReadStep::Progress);
            }
            let n = match self.handle.as_file_mut().read(&mut buf) {
                Ok(n) => n,
                Err(e) => break Err(SourceError::io(&self.path, e)),
            };
            if n == 0 {
                // The end came earlier than the length said (shrunk between the stat and the
                // read): shows as a truncation on the next pass.
                break Ok(ReadStep::Progress);
            }
            self.offset += n as u64;
            self.ingest(&buf[..n], emit);
        };
        self.buf = buf;
        result
    }

    /// Decode a chunk of raw bytes and emit every completed line.
    fn ingest(&mut self, bytes: &[u8], emit: &mut dyn FnMut(String)) {
        self.text.clear();
        let mut text = std::mem::take(&mut self.text);
        self.decoder.feed(bytes, &mut text);
        self.split_text(&text, emit);
        self.text = text;
    }

    /// Split decoded text at newlines into emitted lines and the pending buffer.
    fn split_text(&mut self, text: &str, emit: &mut dyn FnMut(String)) {
        let mut rest: &str = text;
        while let Some(pos) = memchr(b'\n', rest.as_bytes()) {
            let (head, tail) = rest.split_at(pos);
            if self.skip_until_newline {
                // The torn fragment before the start point ends here.
                self.skip_until_newline = false;
                self.pending.clear();
            } else {
                let mut line = std::mem::take(&mut self.pending);
                line.push_str(head);
                if line.ends_with('\r') {
                    line.pop();
                }
                emit(line);
            }
            rest = &tail[1..];
        }
        self.pending.push_str(rest);
    }

    /// Emit everything still held, the decoder's buffers included. For when no more bytes will
    /// ever arrive for this file (a final drain, a truncation, a stop): a torn last line beats
    /// silently losing data.
    pub fn flush_pending(&mut self, emit: &mut dyn FnMut(String)) {
        // The decoder may hold pre-detection bytes or a dangling partial sequence; decode them
        // now (they may even hold whole lines).
        let mut text = std::mem::take(&mut self.text);
        text.clear();
        self.decoder.finish(&mut text);
        if !text.is_empty() {
            self.split_text(&text, emit);
        }
        self.text = text;

        if self.skip_until_newline {
            self.pending.clear();
            return;
        }
        if !self.pending.is_empty() {
            let mut line = std::mem::take(&mut self.pending);
            if line.ends_with('\r') {
                line.pop();
            }
            emit(line);
        }
    }

    /// An in-place truncation: flush what was held, rewind, detect afresh. (A rewrite in place
    /// of the same size or larger is undetectable by length, a limit every poll-based tail
    /// shares.)
    pub fn handle_truncation(&mut self, emit: &mut dyn FnMut(String)) -> Result<(), SourceError> {
        self.flush_pending(emit);
        self.handle.as_file_mut().seek(SeekFrom::Start(0)).map_err(|e| SourceError::io(&self.path, e))?;
        self.offset = 0;
        self.decoder.reset();
        self.skip_until_newline = false;
        Ok(())
    }

    /// Compare what the path points at now with the file held.
    pub fn check_path(&self) -> PathStatus {
        match Handle::from_path(&self.path) {
            Ok(h) if h == self.handle => PathStatus::Same,
            Ok(_) => PathStatus::Replaced,
            Err(_) => PathStatus::Missing,
        }
    }

    /// Read this file to a stable end: done only when everything is consumed and the length has
    /// not moved for `grace`. Covers the classic rotation race, where the writer flushes a final
    /// buffer into the retiring file just after its successor appeared.
    ///
    /// Does not flush the pending partial line: whether the file is truly done (flush) or only
    /// momentarily unreachable (keep holding) is the caller's call.
    pub fn drain_stable(
        &mut self,
        grace: Duration,
        stop: &AtomicBool,
        emit: &mut dyn FnMut(String),
    ) -> Result<(), SourceError> {
        let nap = Duration::from_millis(10);
        let mut last_len = self.disk_len()?;
        let mut stable_since = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            match self.read_new(emit)? {
                ReadStep::Truncated => return Ok(()), // the writer emptied it; nothing more to drain
                ReadStep::Progress => {}
            }
            let len = self.disk_len()?;
            if len != last_len {
                last_len = len;
                stable_since = Instant::now();
            } else if self.offset >= len && stable_since.elapsed() >= grace {
                return Ok(());
            }
            std::thread::sleep(nap);
        }
    }
}
