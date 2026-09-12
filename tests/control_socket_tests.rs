//! Real worker processes, isolated queues and directories; no process-global env mutation.
#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const SOURCE: &str = "import { configure_queue } from \"std/jobs\"\nconfigure_queue(map { \"store\": \"sqlite::memory:\" })\njob Example on test { perform() { print(\"unused\") } }\n";
    fn command(dir: &Path) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        c.current_dir(dir)
            .env("XDG_RUNTIME_DIR", dir)
            .env_remove("NTNT_CONTROL_SOCKET")
            .env_remove("NTNT_WORKER_GROUP")
            .env_remove("NTNT_STRICT")
            .env("NTNT_ENV", "production");
        c
    }
    trait BoundedOutput {
        fn bounded_output(&mut self) -> std::process::Output;
    }
    impl BoundedOutput for Command {
        fn bounded_output(&mut self) -> std::process::Output {
            let out = tempfile::tempfile().unwrap();
            let err = tempfile::tempfile().unwrap();
            let mut child = Worker(
                self.stdout(out.try_clone().unwrap())
                    .stderr(err.try_clone().unwrap())
                    .spawn()
                    .unwrap(),
            );
            wait_for(|| child.0.try_wait().unwrap().is_some());
            use std::io::{Read, Seek};
            let read = |mut file: fs::File| {
                file.rewind().unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                bytes
            };
            std::process::Output {
                status: child.0.wait().unwrap(),
                stdout: read(out),
                stderr: read(err),
            }
        }
    }
    struct Worker(Child);
    impl Drop for Worker {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    impl Worker {
        fn start(dir: &Path, args: &[&str]) -> Self {
            Self(
                command(dir)
                    .args(["worker", "worker.tnt", "--concurrency", "2"])
                    .args(args)
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .unwrap(),
            )
        }
        fn stop(&mut self) {
            unsafe {
                libc::kill(self.0.id() as i32, libc::SIGINT);
            }
            wait_for(|| self.0.try_wait().unwrap().is_some());
        }
    }
    fn wait_for(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !predicate() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    fn project() -> tempfile::TempDir {
        // Control endpoint fixtures need a short runtime path on every Unix.
        // Long source/runtime paths are constructed explicitly in their tests.
        let d = tempfile::tempdir_in("/tmp").unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(d.path().join("worker.tnt"), SOURCE).unwrap();
        d
    }
    fn status(dir: &Path, args: &[&str]) -> bool {
        let mut cmd = command(dir);
        if let Some(i) = args.iter().position(|a| *a == "--dir") {
            cmd.env("XDG_RUNTIME_DIR", args[i + 1]);
        }
        cmd.args(["workers", "status"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .bounded_output()
            .status
            .success()
    }
    #[test]
    fn second_worker_cannot_steal_and_crash_recovers() {
        let d = project();
        let path = d.path().join("control.sock");
        let args = ["--control-socket", path.to_str().unwrap()];
        let mut first = Worker::start(d.path(), &args);
        wait_for(|| status(d.path(), &args));
        let inode = fs::metadata(&path).unwrap().ino();
        let mut second = Worker::start(d.path(), &args);
        wait_for(|| second.0.try_wait().unwrap().is_some());
        assert!(!second.0.wait().unwrap().success());
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        assert!(status(d.path(), &args));
        first.0.kill().unwrap();
        first.0.wait().unwrap();
        let mut recovered = Worker::start(d.path(), &args);
        wait_for(|| status(d.path(), &args));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        recovered.stop();
        assert!(!path.exists());
        assert!(d.path().join("control.sock.lock").exists());
    }
    #[test]
    fn groups_and_source_discovery_from_other_cwd() {
        let d = project();
        fs::write(d.path().join("ntnt.toml"), "").unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut a = Worker::start(d.path(), &["--worker-group", "one"]);
        let mut b = Worker::start(d.path(), &["--worker-group", "two"]);
        for group in ["one", "two"] {
            wait_for(|| {
                status(
                    other.path(),
                    &["--dir", d.path().to_str().unwrap(), "--worker-group", group],
                )
            });
        }
        assert!(!d.path().join(".ntnt.sock").exists());
        a.stop();
        assert!(status(
            other.path(),
            &["--dir", d.path().to_str().unwrap(), "--worker-group", "two"]
        ));
        b.stop();
    }
    #[test]
    fn replacement_file_survives_shutdown() {
        let d = project();
        let path = d.path().join("control.sock");
        let args = ["--control-socket", path.to_str().unwrap()];
        let mut a = Worker::start(d.path(), &args);
        wait_for(|| status(d.path(), &args));
        fs::remove_file(&path).unwrap();
        fs::write(&path, "replacement").unwrap();
        a.stop();
        assert_eq!(fs::read_to_string(path).unwrap(), "replacement");
    }

    #[test]
    fn simultaneous_startup_has_one_winner() {
        let d = project();
        let path = d.path().join("race.sock");
        let args = ["--control-socket", path.to_str().unwrap()];
        let mut a = Worker::start(d.path(), &args);
        let mut b = Worker::start(d.path(), &args);
        wait_for(|| a.0.try_wait().unwrap().is_some() || b.0.try_wait().unwrap().is_some());
        assert_ne!(
            a.0.try_wait().unwrap().is_some(),
            b.0.try_wait().unwrap().is_some()
        );
        assert!(status(d.path(), &args));
    }

    #[test]
    fn unsafe_paths_fail_without_modification() {
        use std::os::unix::fs::symlink;
        for case in [
            "file",
            "socket_symlink",
            "lock_symlink",
            "unsafe_lock",
            "parent",
        ] {
            let d = project();
            let path = d.path().join("control.sock");
            let lock = d.path().join("control.sock.lock");
            let sentinel = d.path().join("sentinel");
            fs::write(&sentinel, "untouched").unwrap();
            match case {
                "file" => fs::write(&path, "untouched").unwrap(),
                "socket_symlink" => symlink(&sentinel, &path).unwrap(),
                "lock_symlink" => symlink(&sentinel, &lock).unwrap(),
                "unsafe_lock" => {
                    fs::write(&lock, "untouched").unwrap();
                    fs::set_permissions(&lock, fs::Permissions::from_mode(0o666)).unwrap();
                }
                "parent" => {
                    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o777)).unwrap()
                }
                _ => unreachable!(),
            }
            let mut worker = Worker::start(d.path(), &["--control-socket", path.to_str().unwrap()]);
            wait_for(|| worker.0.try_wait().unwrap().is_some());
            assert!(!worker.0.wait().unwrap().success(), "{case}");
            assert_eq!(fs::read_to_string(&sentinel).unwrap(), "untouched");
            if case == "file" {
                assert_eq!(fs::read_to_string(&path).unwrap(), "untouched");
            }
            if case == "unsafe_lock" {
                assert_eq!(fs::metadata(&lock).unwrap().mode() & 0o777, 0o666);
            }
            if case.ends_with("symlink") {
                assert!(
                    fs::symlink_metadata(if case == "lock_symlink" { &lock } else { &path })
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
            }
        }
    }

    #[test]
    fn live_noncooperating_listener_and_replacement_socket_are_preserved() {
        use std::os::unix::net::UnixListener;
        let d = project();
        let path = d.path().join("live.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        let mut worker = Worker::start(d.path(), &["--control-socket", path.to_str().unwrap()]);
        wait_for(|| worker.0.try_wait().unwrap().is_some());
        assert!(!worker.0.wait().unwrap().success());
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        drop(listener);
        let mut worker = Worker::start(d.path(), &["--control-socket", path.to_str().unwrap()]);
        wait_for(|| status(d.path(), &["--control-socket", path.to_str().unwrap()]));
        fs::remove_file(&path).unwrap();
        let _replacement = UnixListener::bind(&path).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        worker.stop();
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    }

    #[test]
    fn cli_overrides_environment_and_relative_path_uses_project() {
        let d = project();
        let elsewhere = project();
        let source = d.path().join("worker.tnt");
        let mut worker = Worker(
            command(elsewhere.path())
                .args([
                    "worker",
                    source.to_str().unwrap(),
                    "--concurrency",
                    "2",
                    "--control-socket",
                    "chosen.sock",
                    "--worker-group",
                    "cli",
                ])
                .env("NTNT_CONTROL_SOCKET", "wrong.sock")
                .env("NTNT_WORKER_GROUP", "env")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| d.path().join("chosen.sock").exists());
        let result = command(elsewhere.path())
            .args([
                "workers",
                "status",
                "--dir",
                d.path().to_str().unwrap(),
                "--control-socket",
                "chosen.sock",
            ])
            .env("NTNT_CONTROL_SOCKET", "wrong.sock")
            .bounded_output();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!elsewhere.path().join("chosen.sock").exists());
        assert!(!d.path().join("wrong.sock").exists());
        worker.stop();
    }

    #[test]
    fn embedded_work_jobs_uses_source_context_and_env() {
        let d = project();
        let elsewhere = project();
        fs::write(
            d.path().join("worker.tnt"),
            format!("{SOURCE}\nimport {{ work_jobs }} from \"std/jobs\"\nwork_jobs()\n"),
        )
        .unwrap();
        let source = d.path().join("worker.tnt");
        let mut worker = Worker(
            command(elsewhere.path())
                .args(["run", source.to_str().unwrap()])
                .env("NTNT_CONTROL_SOCKET", "embedded.sock")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| status(d.path(), &["--control-socket", "embedded.sock"]));
        let failure = command(elsewhere.path())
            .args(["run", source.to_str().unwrap()])
            .env("NTNT_CONTROL_SOCKET", "embedded.sock")
            .bounded_output();
        assert!(!failure.status.success());
        assert!(String::from_utf8_lossy(&failure.stderr)
            .contains(d.path().join("embedded.sock").to_str().unwrap()));
        worker.stop();
        assert!(!d.path().join("embedded.sock").exists());
    }

    #[test]
    fn first_use_discovery_and_invalid_cli_are_side_effect_free() {
        let d = project();
        // macOS's normal TMPDIR can exceed sockaddr_un's pathname limit. Keep
        // this first-use XDG fixture short; fallback is a separate behavior.
        let runtime = tempfile::tempdir_in("/tmp").unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let result = command(d.path())
            .env("XDG_RUNTIME_DIR", runtime.path())
            .args(["workers", "status"])
            .bounded_output();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr)
            .contains(runtime.path().join("ntnt").to_str().unwrap()));
        assert_eq!(
            fs::metadata(runtime.path().join("ntnt")).unwrap().uid(),
            unsafe { libc::geteuid() }
        );
        assert_eq!(
            fs::metadata(runtime.path().join("ntnt")).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::read_dir(runtime.path().join("ntnt")).unwrap().count(),
            0
        );
        let result = command(d.path())
            .args([
                "worker",
                "worker.tnt",
                "--control-socket",
                "never.sock",
                "--unknown",
            ])
            .bounded_output();
        assert!(!result.status.success());
        assert!(!d.path().join("never.sock.lock").exists());
        let long = d.path().join(format!("{}.sock", "x".repeat(104)));
        let result = command(d.path())
            .args([
                "worker",
                "worker.tnt",
                "--control-socket",
                long.to_str().unwrap(),
            ])
            .bounded_output();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(long.to_str().unwrap()));
        assert!(!long.exists());
    }

    #[test]
    fn trickling_client_does_not_delay_shutdown() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let d = project();
        let path = d.path().join("slow.sock");
        let args = ["--control-socket", path.to_str().unwrap()];
        let mut worker = Worker::start(d.path(), &args);
        wait_for(|| status(d.path(), &args));
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream.write_all(b"{").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        worker.stop();
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(!path.exists());
    }
    #[test]
    fn embedded_async_options_override_env_and_fail_before_work() {
        let d = project();
        let elsewhere = project();
        let source = d.path().join("worker.tnt");
        fs::write(&source, format!("{SOURCE}\nimport {{ work_async }} from \"std/jobs\"\nimport {{ sleep_ms }} from \"std/concurrent\"\nlet handles = work_async(map {{ \"control_socket\": \"async.sock\", \"concurrency\": 1 }})\nif typeof(handles) == \"Array\" {{ sleep_ms(60000) }}\n")).unwrap();
        let mut worker = Worker(
            command(elsewhere.path())
                .args(["run", source.to_str().unwrap()])
                .env("NTNT_CONTROL_SOCKET", "wrong.sock")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| status(d.path(), &["--control-socket", "async.sock"]));
        let result = command(elsewhere.path())
            .args(["run", source.to_str().unwrap()])
            .env("NTNT_CONTROL_SOCKET", "wrong.sock")
            .bounded_output();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("async.sock"));
        assert!(!d.path().join("wrong.sock").exists());
        worker.stop();
        assert!(!d.path().join("async.sock").exists());
    }

    #[test]
    fn env_group_and_cli_group_precedence() {
        let d = project();
        let mut worker = Worker(
            command(d.path())
                .args([
                    "worker",
                    "worker.tnt",
                    "--worker-group",
                    "chosen",
                    "--concurrency",
                    "2",
                ])
                .env("NTNT_WORKER_GROUP", "wrong")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| status(d.path(), &["--worker-group", "chosen"]));
        let result = command(d.path())
            .args(["workers", "status"])
            .env("NTNT_WORKER_GROUP", "chosen")
            .bounded_output();
        assert!(result.status.success());
        let result = command(d.path())
            .args(["workers", "status", "--worker-group", "chosen"])
            .env("NTNT_WORKER_GROUP", "wrong")
            .bounded_output();
        assert!(result.status.success());
        assert!(!status(d.path(), &["--worker-group", "wrong"]));
        worker.stop();
        let mut worker = Worker(
            command(d.path())
                .args(["worker", "worker.tnt", "--concurrency", "2"])
                .env("NTNT_WORKER_GROUP", "environment")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| status(d.path(), &["--worker-group", "environment"]));
        worker.stop();
    }

    #[test]
    fn cancelling_embedded_blocking_worker_closes_control_before_host_exit() {
        let d = project();
        fs::write(
            d.path().join("worker.tnt"),
            r#"
import { configure_queue, work_jobs } from "std/jobs"
import { spawn, cancel_task, try_await, sleep_ms } from "std/concurrent"
import { write_file, exists } from "std/fs"
configure_queue(map { "store": "sqlite::memory:" })
let handle = spawn(fn() { work_jobs(map { "control_socket": "cancel.sock", "concurrency": 1 }) })
while !exists("cancel-now") { sleep_ms(10) }
cancel_task(handle)
let result = try_await(handle)
write_file("cancelled.txt", "done")
sleep_ms(60000)
"#,
        )
        .unwrap();
        let mut host = Worker(
            command(d.path())
                .args(["run", "worker.tnt"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| status(d.path(), &["--control-socket", "cancel.sock"]));
        fs::write(d.path().join("cancel-now"), "go").unwrap();
        wait_for(|| d.path().join("cancelled.txt").exists());
        wait_for(|| !d.path().join("cancel.sock").exists());
        assert!(host.0.try_wait().unwrap().is_none());
        host.stop();
    }

    #[test]
    fn long_xdg_runtime_uses_the_same_safe_fallback_for_server_and_client() {
        let d = project();
        let long_runtime = d.path().join("r".repeat(140));
        fs::create_dir(&long_runtime).unwrap();
        fs::set_permissions(&long_runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let before = command(d.path())
            .env("XDG_RUNTIME_DIR", &long_runtime)
            .args(["workers", "status"])
            .bounded_output();
        assert!(!before.status.success());
        let fallback = format!("/tmp/ntnt-{}", unsafe { libc::geteuid() });
        assert!(String::from_utf8_lossy(&before.stderr).contains(&fallback));
        assert!(!long_runtime.join("ntnt").exists());
        let mut worker = Worker(
            command(d.path())
                .env("XDG_RUNTIME_DIR", &long_runtime)
                .args(["worker", "worker.tnt", "--concurrency", "2"])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        wait_for(|| {
            command(d.path())
                .env("XDG_RUNTIME_DIR", &long_runtime)
                .args(["workers", "status"])
                .bounded_output()
                .status
                .success()
        });
        assert_eq!(fs::metadata(&fallback).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&fallback).unwrap().uid(), unsafe {
            libc::geteuid()
        });
        assert!(!d.path().join(".ntnt.sock").exists());
        worker.stop();
    }

    #[test]
    fn projects_share_runtime_storage_without_collision_even_with_long_source_paths() {
        let d = project();
        let nested = d.path().join("x".repeat(140));
        fs::create_dir(&nested).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(nested.join("worker.tnt"), SOURCE).unwrap();
        let mut a = Worker::start(d.path(), &[]);
        let mut b = Worker(
            command(d.path())
                .args([
                    "worker",
                    nested.join("worker.tnt").to_str().unwrap(),
                    "--concurrency",
                    "2",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        for project in [d.path(), nested.as_path()] {
            wait_for(|| {
                command(d.path())
                    .args(["workers", "status", "--dir", project.to_str().unwrap()])
                    .bounded_output()
                    .status
                    .success()
            });
        }
        let sockets = fs::read_dir(d.path().join("ntnt"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|e| e == "sock")
            })
            .count();
        assert_eq!(sockets, 2);
        assert!(!nested.join(".ntnt.sock").exists());
        a.stop();
        assert!(command(d.path())
            .args(["workers", "status", "--dir", nested.to_str().unwrap()])
            .bounded_output()
            .status
            .success());
        b.stop();
    }

    #[test]
    fn client_waits_for_live_listener_backlog_to_clear() {
        use std::io::{Read, Write};
        let d = project();
        let path = d.path().join("busy.sock");
        let listener =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        let address = socket2::SockAddr::unix(&path).unwrap();
        listener.bind(&address).unwrap();
        listener.listen(1).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut pending = Vec::new();
        let mut saturated = false;
        for _ in 0..16 {
            let stream =
                socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
            if stream
                .connect_timeout(&address, Duration::from_millis(100))
                .is_err()
                || stream.peer_addr().is_err()
            {
                saturated = true;
                break;
            }
            pending.push(stream);
        }
        assert!(
            saturated && !pending.is_empty(),
            "fixture must fill the real listen backlog"
        );
        let occupied = pending.len();
        listener.set_nonblocking(true).unwrap();
        let listener: std::os::unix::net::UnixListener = listener.into();
        let server = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut drained = 0;
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if drained < occupied {
                            drained += 1;
                            continue;
                        }
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = [0; 256];
                        if stream.read(&mut request).is_ok_and(|n| n > 0) {
                            return stream.write_all(b"{\"bands\":[]}\n").is_ok();
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("accept failed: {e}"),
                }
            }
            false
        });
        let result = command(d.path())
            .args([
                "workers",
                "status",
                "--control-socket",
                path.to_str().unwrap(),
            ])
            .bounded_output();
        let served = server.join().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(served, "client must receive the actual fixture response");
    }

    #[test]
    fn ambiguous_full_backlog_cannot_be_stolen() {
        let d = project();
        let path = d.path().join("backlog.sock");
        let listener =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        listener
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();
        listener.listen(0).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        let mut pending = Vec::new();
        for _ in 0..16 {
            let stream =
                socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
            stream.set_nonblocking(true).unwrap();
            if stream
                .connect(&socket2::SockAddr::unix(&path).unwrap())
                .is_err()
            {
                break;
            }
            pending.push(stream);
        }
        let mut worker = Worker::start(d.path(), &["--control-socket", path.to_str().unwrap()]);
        wait_for(|| worker.0.try_wait().unwrap().is_some());
        assert!(!worker.0.wait().unwrap().success());
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    }
}

#[cfg(windows)]
#[test]
fn windows_control_is_explicitly_unsupported() {
    use ntnt::control_socket::{resolve, start_control_socket, ControlOptions};
    let d = tempfile::tempdir().unwrap();
    let explicit = ControlOptions {
        control_socket: Some(d.path().join("socket")),
        worker_group: None,
    };
    assert_eq!(
        resolve(d.path(), &explicit).unwrap_err().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert!(start_control_socket(&explicit).is_err());
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["workers", "status"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported on Windows"));
}
