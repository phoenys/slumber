"""Root-only test in a disposable Linux runner/container, never production."""
import os
import multiprocessing
import pathlib
import pwd
import socket
import subprocess
import sys
import tempfile


def serve(path, uid, gid, channel):
    if uid:
        os.setgroups([])
        os.setgid(gid)
        os.setuid(uid)
    with socket.socket(socket.AF_UNIX) as listener:
        listener.bind(path)
        os.chmod(path, 0o666)
        listener.listen()
        listener.settimeout(10)
        channel.send("ready")
        with listener.accept()[0] as peer:
            peer.settimeout(10)
            channel.send(peer.recv(1024))


def main():
    binary, username = sys.argv[1:]
    account = pwd.getpwnam(username)
    assert os.geteuid() == 0 and account.pw_uid != 0
    with tempfile.TemporaryDirectory(prefix="slumber-ipc-") as directory:
        root = pathlib.Path(directory)
        os.chown(root, account.pw_uid, account.pw_gid)
        # Independently fail peer credentials and filesystem ownership.
        for server_uid, owner in ((0, account.pw_uid), (account.pw_uid, 0)):
            path = root / "socket"
            parent, child = multiprocessing.Pipe()
            server = multiprocessing.Process(target=serve, args=(str(path), server_uid, account.pw_gid, child))
            server.start()
            try:
                assert parent.poll(10) and parent.recv() == "ready"
                os.chown(path, owner, account.pw_gid)

                def become_client():
                    os.setgroups([])
                    os.setgid(account.pw_gid)
                    os.setuid(account.pw_uid)

                client = subprocess.Popen(
                    [binary, "run", "--no-resume", "true"],
                    env={"PATH": "/usr/bin:/bin", "HOME": account.pw_dir,
                         "SLUMBER_HOME": str(root / "state"),
                         "SLUMBER_SOCKET": str(path), "TEST_SECRET": "must-not-cross-socket"},
                    cwd=directory, preexec_fn=become_client,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
                _, stderr = client.communicate(timeout=10)
                assert parent.poll(10) and parent.recv() == b"", "request leaked to an untrusted socket"
                assert client.returncode != 0 and b"untrusted daemon" in stderr
                server.join(timeout=10)
                assert server.exitcode == 0
            finally:
                if server.is_alive():
                    server.terminate()
                    server.join()
                parent.close()
                child.close()
            path.unlink()
    print("PASS: foreign socket owner and foreign peer UID rejected before any request bytes.")


if __name__ == "__main__":
    main()
