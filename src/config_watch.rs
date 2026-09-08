use crate::config::Config;
use gtk4::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
};

type Reload = Rc<dyn Fn(Config)>;
type Message = Rc<dyn Fn(&str)>;

struct DiskState {
    expected: Option<String>,
    error: Option<String>,
}

pub struct ConfigWatch {
    path: PathBuf,
    disk: Arc<Mutex<DiskState>>,
    pending: RefCell<Option<Config>>,
    save_timer: RefCell<Option<glib::SourceId>>,
    reload_timer: RefCell<Option<glib::SourceId>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    busy: Cell<bool>,
    reading: Cell<bool>,
    reload_again: Cell<bool>,
    notify: Cell<bool>,
    revision: Cell<u64>,
    stopped: Cell<bool>,
    apply: RefCell<Option<Reload>>,
    message: RefCell<Option<Message>>,
}

impl ConfigWatch {
    pub fn new(config: &Config) -> Rc<Self> {
        Rc::new(Self {
            path: config.source.clone(),
            disk: Arc::new(Mutex::new(DiskState {
                expected: config.source_text.clone(),
                error: None,
            })),
            pending: RefCell::new(None),
            save_timer: RefCell::new(None),
            reload_timer: RefCell::new(None),
            monitor: RefCell::new(None),
            busy: Cell::new(false),
            reading: Cell::new(false),
            reload_again: Cell::new(false),
            notify: Cell::new(false),
            revision: Cell::new(0),
            stopped: Cell::new(false),
            apply: RefCell::new(None),
            message: RefCell::new(None),
        })
    }

    pub fn watch(self: &Rc<Self>, apply: Reload, message: Message) {
        *self.apply.borrow_mut() = Some(apply);
        *self.message.borrow_mut() = Some(message);
        let Some(parent) = self.path.parent() else {
            return;
        };
        match gio::File::for_path(parent)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        {
            Ok(monitor) => {
                let weak = Rc::downgrade(self);
                monitor.connect_changed(move |_, file, other, _| {
                    let Some(this) = weak.upgrade() else { return };
                    if file.path().as_ref() == Some(&this.path)
                        || other.and_then(|f| f.path()).as_ref() == Some(&this.path)
                    {
                        this.reload(false);
                    }
                });
                *self.monitor.borrow_mut() = Some(monitor);
            }
            Err(_) => self.report("Could not watch configuration changes"),
        }
    }

    pub fn save(self: &Rc<Self>, config: Config) {
        if self.stopped.get() {
            return;
        }
        self.revision.set(self.revision.get().wrapping_add(1));
        *self.pending.borrow_mut() = Some(config);
        if let Some(timer) = self.save_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        *self.save_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            crate::app::SELF_WRITE_GRACE,
            move || {
                if let Some(this) = weak.upgrade() {
                    this.save_timer.borrow_mut().take();
                    this.start_save();
                }
            },
        ));
    }

    fn start_save(self: &Rc<Self>) {
        if self.stopped.get() || self.busy.get() {
            return;
        }
        let Some(config) = self.pending.borrow_mut().take() else {
            return;
        };
        self.busy.set(true);
        let disk = self.disk.clone();
        let path = self.path.clone();
        let job = gio::spawn_blocking(move || commit(&disk, &path, &config));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = job.await;
            let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) else {
                return;
            };
            this.busy.set(false);
            if !matches!(result, Ok(Ok(()))) {
                this.pending.borrow_mut().take();
                this.report(
                    "Could not save settings; original configuration preserved. Reload and retry.",
                );
                this.reload_again.set(true);
            }
            if this.pending.borrow().is_some() {
                this.start_save();
            } else if this.reload_again.replace(false) {
                this.reload(false);
            }
        });
    }

    pub fn reload(self: &Rc<Self>, notify: bool) {
        if self.stopped.get() {
            return;
        }
        self.notify.set(self.notify.get() || notify);
        if let Some(timer) = self.reload_timer.borrow_mut().take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        *self.reload_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            crate::app::SELF_WRITE_GRACE,
            move || {
                if let Some(this) = weak.upgrade() {
                    this.reload_timer.borrow_mut().take();
                    this.start_reload();
                }
            },
        ));
    }

    fn start_reload(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        if self.busy.get() || self.pending.borrow().is_some() || self.reading.replace(true) {
            self.reload_again.set(true);
            return;
        }
        let revision = self.revision.get();
        let path = self.path.clone();
        let job = gio::spawn_blocking(move || Config::read_from(&path));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = job.await;
            let Some(this) = weak.upgrade().filter(|this| !this.stopped.get()) else {
                return;
            };
            this.reading.set(false);
            if revision != this.revision.get() || this.busy.get() || this.pending.borrow().is_some()
            {
                this.reload_again.set(true);
                this.reload(false);
                return;
            }
            let notify = this.notify.replace(false);
            match result {
                Ok(Ok(config)) => {
                    let changed = {
                        let mut disk = this.disk.lock().unwrap_or_else(|e| e.into_inner());
                        let changed = disk.expected != config.source_text;
                        disk.expected = config.source_text.clone();
                        disk.error = None;
                        changed
                    };
                    if changed || notify {
                        let apply = this.apply.borrow().clone();
                        if let Some(apply) = apply {
                            apply(config);
                        }
                        this.report("Configuration reloaded");
                    }
                }
                _ => this.report("Invalid or unreadable configuration; last good settings kept"),
            }
            if this.reload_again.replace(false) {
                this.reload(false);
            }
        });
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        if self.stopped.replace(true) {
            return Ok(());
        }
        for timer in [&self.save_timer, &self.reload_timer] {
            if let Some(timer) = timer.borrow_mut().take() {
                timer.remove();
            }
        }
        if let Some(monitor) = self.monitor.borrow_mut().take() {
            monitor.cancel();
        }
        self.apply.borrow_mut().take();
        self.message.borrow_mut().take();
        if let Some(config) = self.pending.borrow_mut().take() {
            commit(&self.disk, &self.path, &config)?;
        }
        let disk = self.disk.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &disk.error {
            anyhow::bail!("{error}");
        }
        Ok(())
    }

    fn report(&self, text: &str) {
        let message = self.message.borrow().clone();
        if let Some(message) = message {
            message(text);
        }
    }
}

fn commit(disk: &Mutex<DiskState>, path: &std::path::Path, config: &Config) -> anyhow::Result<()> {
    let mut disk = disk.lock().unwrap_or_else(|e| e.into_inner());
    match config.write_checked(path, disk.expected.as_deref()) {
        Ok(text) => {
            disk.expected = Some(text);
            disk.error = None;
            Ok(())
        }
        Err(error) => {
            disk.error =
                Some("Could not persist settings; configuration file was preserved".into());
            Err(error)
        }
    }
}

impl Drop for ConfigWatch {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gtk4::test]
    fn coalesces_writes_and_flushes_latest_snapshot() {
        let dir = crate::test_support::TestDir::new("config-watch");
        let path = dir.path().join("config.toml");
        Config::default().write_to(&path).unwrap();
        let mut config = Config::read_from(&path).unwrap();
        let watch = ConfigWatch::new(&config);
        for size in 14..20 {
            config.font_size = size as f32;
            watch.save(config.clone());
        }
        watch.stop().unwrap();
        assert_eq!(Config::read_from(&path).unwrap().font_size, 19.0);
    }

    #[gtk4::test]
    fn external_edits_are_not_overwritten_by_pending_settings() {
        let dir = crate::test_support::TestDir::new("config-conflict");
        let path = dir.path().join("config.toml");
        Config::default().write_to(&path).unwrap();
        let mut config = Config::read_from(&path).unwrap();
        let watch = ConfigWatch::new(&config);
        config.font_size = 20.0;
        watch.save(config);
        std::fs::write(&path, "[font]\nsize = 17\n").unwrap();
        assert!(watch.stop().is_err());
        assert_eq!(Config::read_from(&path).unwrap().font_size, 17.0);
    }
}
