/// Trace Recorder Module
///
/// Provides a trait and implementations for flushing energy trace data to disk.
/// The `CsvTraceRecorder` writes data from a `RotatingTrace` to CSV files with
/// automatic file rotation based on size limits.
use crate::utils::trace_rotation::RotatingTrace;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// Trait for recording trace data to persistent storage.
///
/// Implementors receive a reference to a `RotatingTrace` and are responsible
/// for writing some or all of that data to their backing store.
pub trait TraceRecorder: Send + Sync {
    /// Flush data from the given trace to persistent storage.
    fn flush(&mut self, trace: &RotatingTrace);
}

/// A CSV-based trace recorder that writes energy records to rotating CSV files.
///
/// Behavior:
/// - Writes all columns from the RotatingTrace DataFrame (pid, timestamp, device, energy).
/// - Rotates to a new file when the current file exceeds `max_file_size_bytes`.
/// - Keeps at most `max_files` CSV files, deleting the oldest when the limit is exceeded.
/// - Only flushes records newer than the last flushed timestamp to avoid duplicates.
pub struct CsvTraceRecorder {
    output_dir: PathBuf,
    max_file_size_bytes: u64,
    max_files: usize,
    current_file: Option<File>,
    current_file_path: Option<PathBuf>,
    current_file_size: u64,
    file_index: usize,
    last_flushed_timestamp: Option<i64>,
}

impl CsvTraceRecorder {
    /// Create a new CsvTraceRecorder.
    ///
    /// # Arguments
    /// * `output_dir` - Directory where CSV files will be written.
    /// * `max_file_size_bytes` - Maximum size per file before rotation (default: 10 MB).
    /// * `max_files` - Maximum number of rotated files to keep (default: 5).
    pub fn new(
        output_dir: PathBuf,
        max_file_size_bytes: Option<u64>,
        max_files: Option<usize>,
    ) -> Self {
        Self {
            output_dir,
            max_file_size_bytes: max_file_size_bytes.unwrap_or(10 * 1024 * 1024),
            max_files: max_files.unwrap_or(5),
            current_file: None,
            current_file_path: None,
            current_file_size: 0,
            file_index: 0,
            last_flushed_timestamp: None,
        }
    }

    /// Generate the file path for a given index.
    fn file_path_for_index(&self, index: usize) -> PathBuf {
        self.output_dir.join(format!("trace_{}.csv", index))
    }

    /// Ensure the output directory exists.
    fn ensure_output_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.output_dir)
    }

    /// Open a new CSV file and write the header row.
    fn rotate_file(&mut self) -> std::io::Result<()> {
        // Close current file if open
        self.current_file = None;

        // Advance to next file index
        self.file_index += 1;
        let path = self.file_path_for_index(self.file_index);

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;

        self.current_file = Some(file);
        self.current_file_path = Some(path);
        self.current_file_size = 0;

        // Write CSV header
        self.write_header()?;

        // Enforce max_files limit
        self.enforce_max_files();

        Ok(())
    }

    /// Write the CSV header row.
    fn write_header(&mut self) -> std::io::Result<()> {
        let header = "pid,timestamp,device,energy\n";
        if let Some(ref mut file) = self.current_file {
            file.write_all(header.as_bytes())?;
            self.current_file_size += header.len() as u64;
        }
        Ok(())
    }

    /// Open the initial file if none is currently open.
    fn ensure_file_open(&mut self) -> std::io::Result<()> {
        if self.current_file.is_none() {
            self.ensure_output_dir()?;
            let path = self.file_path_for_index(self.file_index);

            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)?;

            self.current_file = Some(file);
            self.current_file_path = Some(path);
            self.current_file_size = 0;

            self.write_header()?;
        }
        Ok(())
    }

    /// Remove old files that exceed the max_files limit.
    fn enforce_max_files(&self) {
        if self.file_index < self.max_files {
            return;
        }

        // Delete files older than (file_index - max_files)
        let oldest_to_keep = self.file_index - self.max_files + 1;
        for i in 0..oldest_to_keep {
            let path = self.file_path_for_index(i);
            let _ = fs::remove_file(path);
        }
    }

    /// Write a single CSV row.
    fn write_row(
        &mut self,
        pid: u32,
        timestamp: i64,
        device: &str,
        energy: f64,
    ) -> std::io::Result<()> {
        let row = format!("{},{},{},{}\n", pid, timestamp, device, energy);
        let row_bytes = row.as_bytes();

        // Check if we need to rotate before writing
        if self.current_file_size + row_bytes.len() as u64 > self.max_file_size_bytes {
            self.ensure_output_dir()?;
            self.rotate_file()?;
        }

        if let Some(ref mut file) = self.current_file {
            file.write_all(row_bytes)?;
            self.current_file_size += row_bytes.len() as u64;
        }

        Ok(())
    }
}

impl TraceRecorder for CsvTraceRecorder {
    fn flush(&mut self, trace: &RotatingTrace) {
        let df = trace.data();

        if df.is_empty() {
            return;
        }

        // Extract columns from the DataFrame
        let pid_col = match df.column("pid") {
            Ok(col) => col,
            Err(e) => {
                log::error!("Failed to get 'pid' column from trace: {}", e);
                return;
            }
        };
        let timestamp_col = match df.column("timestamp") {
            Ok(col) => col,
            Err(e) => {
                log::error!("Failed to get 'timestamp' column from trace: {}", e);
                return;
            }
        };
        let device_col = match df.column("device") {
            Ok(col) => col,
            Err(e) => {
                log::error!("Failed to get 'device' column from trace: {}", e);
                return;
            }
        };
        let energy_col = match df.column("energy") {
            Ok(col) => col,
            Err(e) => {
                log::error!("Failed to get 'energy' column from trace: {}", e);
                return;
            }
        };

        // Get typed column iterators
        let pids = match pid_col.u32() {
            Ok(ca) => ca,
            Err(e) => {
                log::error!("Failed to cast 'pid' column to u32: {}", e);
                return;
            }
        };
        let timestamps = match timestamp_col.i64() {
            Ok(ca) => ca,
            Err(e) => {
                log::error!("Failed to cast 'timestamp' column to i64: {}", e);
                return;
            }
        };
        let devices = match device_col.str() {
            Ok(ca) => ca,
            Err(e) => {
                log::error!("Failed to cast 'device' column to str: {}", e);
                return;
            }
        };
        let energies = match energy_col.f64() {
            Ok(ca) => ca,
            Err(e) => {
                log::error!("Failed to cast 'energy' column to f64: {}", e);
                return;
            }
        };

        // Ensure we have an open file
        if let Err(e) = self.ensure_file_open() {
            log::error!("Failed to open trace output file: {}", e);
            return;
        }

        let mut max_timestamp = self.last_flushed_timestamp;

        for row_idx in 0..df.height() {
            let ts = match timestamps.get(row_idx) {
                Some(v) => v,
                None => continue,
            };

            // Skip records we have already flushed
            if let Some(last_ts) = self.last_flushed_timestamp {
                if ts <= last_ts {
                    continue;
                }
            }

            let pid = match pids.get(row_idx) {
                Some(v) => v,
                None => continue,
            };
            let device = match devices.get(row_idx) {
                Some(v) => v,
                None => continue,
            };
            let energy = match energies.get(row_idx) {
                Some(v) => v,
                None => continue,
            };

            if let Err(e) = self.write_row(pid, ts, device, energy) {
                log::error!("Failed to write trace row: {}", e);
                return;
            }

            // Track the maximum timestamp we flushed
            max_timestamp = Some(match max_timestamp {
                Some(current_max) => current_max.max(ts),
                None => ts,
            });
        }

        // Update last flushed timestamp
        if max_timestamp != self.last_flushed_timestamp {
            self.last_flushed_timestamp = max_timestamp;
        }

        // Flush the file to disk
        if let Some(ref mut file) = self.current_file {
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::trace_rotation::RotatingTrace;
    use polars::df;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    fn current_timestamp_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn make_trace_with_data(timestamps: Vec<i64>) -> RotatingTrace {
        let mut trace = RotatingTrace::new(3600);
        let n = timestamps.len();
        let data = df![
            "pid" => vec![1u32; n],
            "timestamp" => timestamps,
            "device" => vec!["cpu".to_string(); n],
            "energy" => vec![10.0; n],
        ]
        .unwrap();
        trace.append(&data).unwrap();
        trace
    }

    #[test]
    fn csv_recorder_creates_file_on_flush() {
        let tmp_dir = TempDir::new().unwrap();
        let mut recorder = CsvTraceRecorder::new(tmp_dir.path().to_path_buf(), None, None);

        let now = current_timestamp_secs();
        let trace = make_trace_with_data(vec![now]);

        recorder.flush(&trace);

        let file_path = tmp_dir.path().join("trace_0.csv");
        assert!(file_path.exists(), "CSV file should be created on flush");
    }

    #[test]
    fn csv_recorder_writes_valid_csv() {
        let tmp_dir = TempDir::new().unwrap();
        let mut recorder = CsvTraceRecorder::new(tmp_dir.path().to_path_buf(), None, None);

        let now = current_timestamp_secs();
        let mut trace = RotatingTrace::new(3600);
        let data = df![
            "pid" => vec![42u32, 43u32],
            "timestamp" => vec![now, now + 1],
            "device" => vec!["cpu".to_string(), "gpu".to_string()],
            "energy" => vec![1.5, 2.5],
        ]
        .unwrap();
        trace.append(&data).unwrap();

        recorder.flush(&trace);

        let file_path = tmp_dir.path().join("trace_0.csv");
        let contents = fs::read_to_string(file_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();

        assert_eq!(lines[0], "pid,timestamp,device,energy");
        assert_eq!(lines.len(), 3); // header + 2 data rows

        // Verify first data row
        let fields: Vec<&str> = lines[1].split(',').collect();
        assert_eq!(fields[0], "42");
        assert_eq!(fields[2], "cpu");
        assert_eq!(fields[3], "1.5");

        // Verify second data row
        let fields: Vec<&str> = lines[2].split(',').collect();
        assert_eq!(fields[0], "43");
        assert_eq!(fields[2], "gpu");
        assert_eq!(fields[3], "2.5");
    }

    #[test]
    fn csv_recorder_rotates_files() {
        let tmp_dir = TempDir::new().unwrap();
        // Use a very small max file size to trigger rotation
        let mut recorder = CsvTraceRecorder::new(tmp_dir.path().to_path_buf(), Some(50), Some(10));

        let now = current_timestamp_secs();
        let mut trace = RotatingTrace::new(3600);
        let data = df![
            "pid" => vec![1u32, 2u32, 3u32, 4u32, 5u32],
            "timestamp" => vec![now, now + 1, now + 2, now + 3, now + 4],
            "device" => vec!["cpu".to_string(); 5],
            "energy" => vec![10.0; 5],
        ]
        .unwrap();
        trace.append(&data).unwrap();

        recorder.flush(&trace);

        // With a 50-byte limit, the header alone is ~25 bytes, so rows should
        // cause rotation. Check that multiple files were created.
        let csv_files: Vec<_> = fs::read_dir(tmp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "csv"))
            .collect();

        assert!(
            csv_files.len() > 1,
            "Expected multiple files after rotation, got {}",
            csv_files.len()
        );
    }

    #[test]
    fn csv_recorder_respects_max_files() {
        let tmp_dir = TempDir::new().unwrap();
        // Very small file size + max 2 files
        let mut recorder = CsvTraceRecorder::new(tmp_dir.path().to_path_buf(), Some(40), Some(2));

        let now = current_timestamp_secs();
        let mut trace = RotatingTrace::new(3600);
        let data = df![
            "pid" => vec![1u32; 10],
            "timestamp" => (0..10).map(|i| now + i).collect::<Vec<_>>(),
            "device" => vec!["cpu".to_string(); 10],
            "energy" => vec![99.9; 10],
        ]
        .unwrap();
        trace.append(&data).unwrap();

        recorder.flush(&trace);

        // Count remaining CSV files
        let csv_files: Vec<_> = fs::read_dir(tmp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "csv"))
            .collect();

        assert!(
            csv_files.len() <= 2,
            "Expected at most 2 files, got {}",
            csv_files.len()
        );
    }

    #[test]
    fn csv_recorder_skips_empty_trace() {
        let tmp_dir = TempDir::new().unwrap();
        let mut recorder = CsvTraceRecorder::new(tmp_dir.path().to_path_buf(), None, None);

        let trace = RotatingTrace::new(3600); // empty trace

        recorder.flush(&trace);

        // No file should be created for an empty trace
        let entries: Vec<_> = fs::read_dir(tmp_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();

        assert_eq!(
            entries.len(),
            0,
            "No files should be created for empty trace"
        );
    }

    #[test]
    fn csv_recorder_only_flushes_new_records() {
        let tmp_dir = TempDir::new().unwrap();
        let mut recorder = CsvTraceRecorder::new(tmp_dir.path().to_path_buf(), None, None);

        let now = current_timestamp_secs();

        // First flush with initial data
        let mut trace = RotatingTrace::new(3600);
        let data1 = df![
            "pid" => vec![1u32, 2u32],
            "timestamp" => vec![now, now + 1],
            "device" => vec!["cpu".to_string(); 2],
            "energy" => vec![10.0; 2],
        ]
        .unwrap();
        trace.append(&data1).unwrap();

        recorder.flush(&trace);

        // Add more data to the same trace (simulating new records arriving)
        let data2 = df![
            "pid" => vec![3u32],
            "timestamp" => vec![now + 2],
            "device" => vec!["cpu".to_string()],
            "energy" => vec![30.0],
        ]
        .unwrap();
        trace.append(&data2).unwrap();

        // Flush again - should only write the new record
        recorder.flush(&trace);

        let file_path = tmp_dir.path().join("trace_0.csv");
        let contents = fs::read_to_string(file_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();

        // header + 2 original records + 1 new record = 4 lines
        assert_eq!(
            lines.len(),
            4,
            "Expected 4 lines (header + 3 records), got {}",
            lines.len()
        );

        // Verify the third data row is the new record
        let fields: Vec<&str> = lines[3].split(',').collect();
        assert_eq!(fields[0], "3");
        assert_eq!(fields[3], "30");
    }
}
