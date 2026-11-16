#!/usr/bin/env python3
"""Boot a prepared nixify test VM with an isolated disk and collect a smoke log.

This runner requires the network- and share-free VM produced by vm-test.nix.
It never invokes sudo or maps a physical disk or host directory into the guest.
"""
import argparse
import os
import pathlib
import pty
import re
import select
import subprocess
import sys
import time


def plain_console(buffer):
    """Remove terminal title/color escapes and normalize carriage returns."""
    clean = re.sub(rb"\x1b\][^\x07]*\x07", b"", buffer)
    return re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", clean).replace(b"\r", b"")


def guest_prompt(buffer):
    return bool(re.search(rb"\[root@nixify-test:[^\]]*\][#$] ", plain_console(buffer)))


def test_done(buffer):
    return b"\nNIXIFY_VM_TEST_DONE\n" in plain_console(buffer)


def command_name(value):
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.+-]{0,127}", value):
        raise argparse.ArgumentTypeError("expected a command name without a path or shell syntax")
    return value


def missing_commands(buffer, expected):
    lines = set(plain_console(buffer).decode("utf-8", errors="replace").splitlines())
    return [name for name in expected if f"NIXIFY_COMMAND_{name}=present" not in lines]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("runner", type=pathlib.Path)
    parser.add_argument("workdir", type=pathlib.Path)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--expect-command", type=command_name, action="append", default=[],
                        help="require this executable in the guest (repeatable)")
    args = parser.parse_args()
    runner = args.runner.resolve(strict=True)
    content = runner.read_text()
    if "-nic none" not in content or "-virtfs" in content or "-netdev" in content or "-net " in content:
        raise SystemExit("Refusing VM runner: expected networking disabled and no host shares")
    workdir = args.workdir.resolve(strict=True)
    scratch = workdir / "runtime"
    scratch.mkdir(mode=0o700)  # Fail if anything exists; never reuse a disk.
    disk = scratch / "guest.qcow2"
    log_path = scratch / "console.log"
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(("QEMU_", "NIXPKGS_QEMU_KERNEL_")) or key in ("SHARED_DIR", "USE_TMPDIR", "NIX_DISK_IMAGE"):
            env.pop(key, None)
    env.update({"NIX_DISK_IMAGE": str(disk), "TMPDIR": str(scratch), "USE_TMPDIR": "1"})
    if not os.access("/dev/kvm", os.R_OK | os.W_OK):
        env["QEMU_OPTS"] = "-accel tcg"
    master, slave = pty.openpty()
    with os.fdopen(os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as log:
        proc = subprocess.Popen([str(runner)], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=scratch, start_new_session=True)
        os.close(slave)
        start = time.monotonic()
        buffer = b""
        sent = False
        saw_marker = False
        try:
            while time.monotonic() - start < args.timeout:
                ready, _, _ = select.select([master], [], [], 1)
                if ready:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        chunk = b""
                    if chunk:
                        log.write(chunk)
                        log.flush()
                        buffer = (buffer + chunk)[-256000:]
                        # Remove terminal colors and title sequences before matching.
                        clean = plain_console(buffer)
                        if not sent and guest_prompt(buffer):
                            commands_to_check = list(dict.fromkeys(["firefox", "nvim", "htop", *args.expect_command]))
                            command_words = " ".join(commands_to_check)  # command_name permits only shell-safe names
                            commands = (
                                "printf '\\nNIXIFY_VM_BOOT_OK\\n'; "
                                f"for cmd in {command_words}; do "
                                "if command -v \"$cmd\" >/dev/null; then printf 'NIXIFY_COMMAND_%s=present\\n' \"$cmd\"; "
                                "else printf 'NIXIFY_COMMAND_%s=absent\\n' \"$cmd\"; fi; done; "
                                "printf 'NIXIFY_SYSTEM_STATE='; systemctl is-system-running || true; "
                                "printf 'NIXIFY_VM_TEST_DONE\\n'; poweroff -f\n"
                            )
                            os.write(master, commands.encode())
                            sent = True
                        if sent and test_done(buffer):
                            saw_marker = True
                code = proc.poll()
                if code is not None:
                    if code == 0 and saw_marker:
                        missing = missing_commands(buffer, args.expect_command)
                        if missing:
                            raise RuntimeError(f"expected guest commands missing: {', '.join(missing)}; log: {log_path}")
                        print(f"VM boot and console smoke test passed; console log: {log_path}")
                        for line in clean.decode("utf-8", errors="replace").splitlines():
                            if line.startswith(("NIXIFY_COMMAND_", "NIXIFY_SYSTEM_STATE=")):
                                print(line)
                        return 0
                    raise RuntimeError(f"VM exited with status {code}; marker observed: {saw_marker}; log: {log_path}")
            raise TimeoutError(f"VM did not finish within {args.timeout} seconds; log: {log_path}")
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, 15)
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, 9)
                    proc.wait()
            os.close(master)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, TimeoutError) as error:
        print(f"VM smoke test failed: {error}", file=sys.stderr)
        sys.exit(1)
