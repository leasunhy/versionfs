VersionFS
=========

A small utility FUSE filesystem written in Rust that helps capture
the changes of a specific file.

It works by backing up the versions across `close-write` boundaries, similar to

```bash
mkdir versions || true
inotifywait -m . -e close_write |
    while read path action file; do
        if [[ "$file" = "target.txt" ]]; then
            i=$((i+1))
            echo $i
            cp $path/$file versions/$i.txt
        fi
    done
```

... but in a blocking fashion, thus avoiding race conditions.


## Usage

```bash
cargo build --release
mkdir backups mountpoint
target/release/versionfs --target target.txt --target_dir backups/ mountpoint/
```

Give `mountpoint/target.txt` to the program as the output path, and the captured
versions would be saved to `backups/`.

In cases where the file needs to be at a specific path, a symlink would be helpful.


## Scenario and Rationale

Say we have a blackbox (close-sourced) program that runs iterative computations
and saves the intermediate results to a dedicated file.
Now we want every intermediate version of the file.
However, the program does not provide an option for saving intermediate results to 
separate files. What gives?

Aditional conditions to consider:

* The program runs at arbitrary speed (depending on workload size).
* The program is a multi-process one.

Possible solutions:

1. `inotifywait`. This watches the file-related events through the `inotify` syscall.
    It seems working, but its reliability is to be questioned when the events come
    really fast. In that case, consecutive events will be merged into one and thus
    we may fail obtaining EVERY version of the file.
2. A PIPE file. This would not work because ORCA not only writes to the GBW, but
    also reads from it.
3. `ptrace`. This is not very feasible if the program is a multi-process one
    (which, in our case, is true).
4. Syscall hooks. For example, hook the `open` syscall by setting `LD_PRELOAD`.
    This should work, but is somewhat tricky because there are multiple `open` like
    syscalls (e.g. `open`, `open64`, `creat`, `openat`) and chances are that
    the program does not respect `LD_PRELOAD`.
5. A custom FUSE filesystem. This is similar to syscall hooks but is less hacky.

This repo is the implementation of the option `5`.


## License

MIT.
