use std::env;
use std::path::PathBuf;
use std::ffi::{OsStr, OsString, CString};
use std::time::{Duration, UNIX_EPOCH};
use std::fs;
use std::os::unix::fs::MetadataExt;

use log::info;
use clap::{crate_version, arg, value_parser, Command};
use libc::{
    c_int, c_void,
    ENOENT, ENOSYS, EEXIST,
    O_WRONLY, O_RDWR, O_TRUNC, O_CREAT,
};
use fuser::{
    Filesystem,
    Request, ReplyEntry, ReplyDirectory, ReplyData, ReplyAttr,
    ReplyOpen, ReplyLseek, ReplyWrite,
    FileType, FileAttr,
};

const TTL: Duration = Duration::from_secs(1);

const PARENT_ATTR: FileAttr = FileAttr {
    ino: 1,
    size: 0,
    blocks: 0,
    atime: UNIX_EPOCH, // 1970-01-01 00:00:00
    mtime: UNIX_EPOCH,
    ctime: UNIX_EPOCH,
    crtime: UNIX_EPOCH,
    kind: FileType::Directory,
    perm: 0o755,
    nlink: 2,
    uid: 501,
    gid: 20,
    rdev: 0,
    flags: 0,
    blksize: 512,
};

#[inline(always)]
fn errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

struct VersionFS {
    /// ino: 1 root, 2 target, 3.. ino in default_dir
    target: OsString,
    target_dir: PathBuf,
    version: usize,
}

impl VersionFS {
    fn path_for_version(&self, version: usize) -> PathBuf {
        let filename = format!("{}.{}", version, self.target.to_str().unwrap());
        self.target_dir.join(filename)
    }

    fn target_attr(&self, version: usize) -> Option<FileAttr> {
        match version {
            v if v > 0 => {
                let size = fs::metadata(self.path_for_version(v))
                    .and_then(|m| Ok(m.size()));
                if let Ok(size) = size {
                    Some(FileAttr {
                        ino: 2,
                        size: size,
                        blocks: 1,
                        atime: UNIX_EPOCH, // 1970-01-01 00:00:00
                        mtime: UNIX_EPOCH,
                        ctime: UNIX_EPOCH,
                        crtime: UNIX_EPOCH,
                        kind: FileType::RegularFile,
                        perm: 0o777,
                        nlink: 1,
                        uid: 501,
                        gid: 20,
                        rdev: 0,
                        flags: 0,
                        blksize: 512,
                    })
                } else {
                    None
                }
            },
            _ => None,
        }
    }

    fn current_target_attr(&self) -> Option<FileAttr> { self.target_attr(self.version) }
}

impl Filesystem for VersionFS {
    fn init(&mut self, _req: &Request, _config: &mut fuser::KernelConfig) -> Result<(), c_int> {
        self.version = 1;
        let path = self.path_for_version(self.version);
        fs::write(path, &[]).unwrap();
        Ok(())
    }

    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        info!("lookup {parent} {name:?}");
        info!("self.version = {}", self.version);
        if parent == 1 && name == self.target {
            let attr =
                self.target_attr(self.version)
                    .or(self.target_attr(self.version - 1))
                    .unwrap();
            reply.entry(&TTL, &attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        info!("getattr {ino}");
        match ino {
            1 => reply.attr(&TTL, &PARENT_ATTR),
            2 if self.version > 0 => reply.attr(&TTL, &self.current_target_attr().unwrap()),
            _ => reply.error(ENOENT),
        }
    }

    fn mknod(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        info!("mknod {parent} {name:?}");
        if parent == 1 && name == self.target {
            reply.error(EEXIST);
        } else {
            reply.error(ENOSYS);
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        info!("read {_fh}");
        if ino == 2 && self.version > 0 {
            let path = self.path_for_version(self.version);
            let data = fs::read(path).unwrap();
            let start = offset as usize;
            let end: usize = data.len().min(start + size as usize);
            reply.data(&data[start..end]);
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        info!("readdir {ino} {_fh}");
        if ino != 1 {
            reply.error(ENOENT);
            return;
        }

        let mut entries = vec![
            (1, FileType::Directory, "."),
            (1, FileType::Directory, ".."),
        ];

        if self.version > 0 {
            entries.push(
                (2, FileType::RegularFile, self.target.to_str().unwrap())
            );
        }

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(entry.0, (i + 1) as i64, entry.1, entry.2) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        info!("open {ino} {flags:b}");
        match ino {
            2 => {
                if flags & O_WRONLY != 0 || flags & O_RDWR != 0 || flags & O_CREAT != 0 {
                    self.version += 1;
                    let newpath = self.path_for_version(self.version);
                    if self.version > 1 && flags & O_TRUNC == 0 {
                        let oldpath = self.path_for_version(self.version - 1);
                        fs::copy(oldpath, newpath).unwrap();
                    } else {
                        fs::write(newpath, &[]).unwrap();
                    }
                }
                let path = self.path_for_version(self.version);
                let cpath = CString::new(path.to_str().unwrap()).unwrap();
                match unsafe { libc::open(cpath.as_ptr(), flags) } {
                    -1 => reply.error(errno()),
                    fd => reply.opened(fd.try_into().unwrap(), flags.try_into().unwrap()),
                };
            },
            _ => reply.error(ENOSYS),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        info!("release {fh} {flags:b}");
        unsafe { libc::close(fh as i32); }
        reply.ok();
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        info!("write {ino} {fh} {offset} {flags:b}");
        let buf = data.as_ptr() as *const c_void;
        match unsafe { libc::pwrite(fh as i32, buf, data.len(), offset) } {
            -1 => reply.error(errno()),
            ret => reply.written(ret as u32),
        }
    }

    fn lseek(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        whence: i32,
        reply: ReplyLseek,
    ) {
        info!("lseek {ino} {fh} {offset} {whence}");
        match unsafe { libc::lseek(fh as i32, offset, whence) } {
            -1 => reply.error(errno()),
            ret => reply.offset(ret),
        }
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        info!("setattr");
        reply.attr(&TTL, &self.current_target_attr().unwrap());
    }
}

fn main() {
    let matches = Command::new("versionfs")
        .version(crate_version!())
        .author("Hmm")
        .arg(
            arg!(<MOUNT_POINT> "Where the FUSE should be mounted")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            arg!(-t --target <FILE> "The target file to be versioned")
                .required(true)
                .value_parser(value_parser!(OsString)),
        )
        .arg(
            arg!(-o --target_dir <DIR> "Where the versions of the target file should be saved")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .get_matches();

    env_logger::init();
    let fs = VersionFS{
        target: matches.get_one::<OsString>("target").unwrap().clone(),
        target_dir: matches.get_one::<PathBuf>("target_dir").unwrap().clone(),
        version: 0,
    };
    let mountpoint = matches.get_one::<PathBuf>("MOUNT_POINT").unwrap();

    let mut daemon = fuser::spawn_mount2(fs, mountpoint, &[]).ok();

    ctrlc::set_handler(move || {
        std::mem::drop(daemon.take());
        std::process::exit(0);
    }).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(10));
    }
}
