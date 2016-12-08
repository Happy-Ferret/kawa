use std::mem;
use hyper::client::Client;
use rustc_serialize::json;
use std::sync::Arc;
use std::io::Read;
use config::Config;
use prebuffer::PreBuffer;
use util;
use slog::Logger;

pub struct Queue {
    pub next: Option<Vec<PreBuffer>>,
    pub entries: Vec<QueueEntry>,
    counter: usize,
    cfg: Config,
    log: Logger,
}

impl Queue {
    pub fn new(cfg: Config, log: Logger) -> Queue {
        Queue {
            next: None,
            entries: Vec::new(),
            cfg: cfg,
            log: log,
            counter: 0,
        }
    }

    pub fn push(&mut self, qe: QueueEntry) {
        debug!(self.log, "Inserting {:?} into queue tail!", qe);
        self.entries.push(qe);
        if self.entries.len() == 1 {
            self.start_next_tc();
        }
    }

    pub fn push_head(&mut self, qe: QueueEntry) {
        debug!(self.log, "Inserting {:?} into queue head!", qe);
        self.entries.insert(0, qe);
        self.start_next_tc();
    }

    pub fn pop(&mut self) {
        debug!(self.log, "Removing {:?} from queue tail!", self.entries.pop());
        if self.entries.len() == 0 {
            self.start_next_tc();
        }
    }

    pub fn pop_head(&mut self) {
        let res = if !self.entries.is_empty() {
            Some(self.entries.remove(0))
        } else {
            None
        };
        debug!(self.log, "Removing {:?} from queue head!", res);
        self.start_next_tc();
    }

    pub fn clear(&mut self) {
        debug!(self.log, "Clearing queue!");
        self.entries.clear();
        self.start_next_tc();
    }

    pub fn get_next_tc(&mut self) -> Vec<PreBuffer> {
        debug!(self.log, "Extracting current pre-transcode!");
        if self.next.is_none() {
            self.start_next_tc();
        }
        return mem::replace(&mut self.next, None).unwrap();
    }

    pub fn start_next_tc(&mut self) {
        debug!(self.log, "Beginning next pre-transcode!");
        loop {
            if let Some(pbs) = self.initiate_transcode() {
                self.next = Some(pbs);
                return;
            }
        }
    }

    fn next_buffer(&mut self) -> (Arc<Vec<u8>>, String) {
        let mut buf = self.next_queue_buffer();
        let mut tries = 10;
        while buf.is_none() {
            if tries == 0 {
                warn!(self.log, "Using fallback song!");
                return self.cfg.queue.fallback.clone();
            }
            buf = self.random_buffer();
            tries -= 1;
        }
        buf.unwrap()
    }

    fn next_queue_buffer(&mut self) -> Option<(Arc<Vec<u8>>, String)> {
        while !self.entries.is_empty() {
            {
                let entry = &self.entries[0];
                if let Some(r) = util::path_to_data(&entry.path) {
                    info!(self.log, "Using queue entry {:?}", entry.path);
                    return Some(r);
                }
            }
            self.entries.remove(0);
        }
        return None;
    }

    fn random_buffer(&mut self) -> Option<(Arc<Vec<u8>>, String)> {
        let client = Client::new();

        let mut body = String::new();
        let mut path = String::new();
        let res = client.get(self.cfg.queue.random.clone())
            .send()
            .ok()
            .and_then(|mut r| r.read_to_string(&mut body).ok())
            .and_then(|_| json::decode(&body).ok())
            .and_then(|e: QueueEntry| {
                debug!(self.log, "Attempting to use random buffer from path {:?}", e.path);
                path = e.path.clone();
                util::path_to_data(&e.path)
            });
        if res.is_some() {
            info!(self.log, "Using random entry {:?}", path);
        }
        res
    }

    fn initiate_transcode(&mut self) -> Option<Vec<PreBuffer>> {
        let (data, ext) = self.next_buffer();
        let mut prebufs = Vec::new();
        debug!(self.log, "Attempting to spawn transcoders with ID: {:?}", self.counter);
        for stream in self.cfg.streams.iter() {
            let slog = self.log.new(o!(
                    "Transcoder, mount" => stream.mount.clone(),
                    "QID" => self.counter
            ));
            if let Some(prebuf) = PreBuffer::from_transcode(data.clone(), &ext, stream, slog) {
                prebufs.push(prebuf);
            } else {
                // Terminate if any prebuffers fail to create.
                return None;
            }
        }
        self.counter += 1;
        Some(prebufs)
    }
}

#[derive(Debug, RustcDecodable, RustcEncodable)]
pub struct QueueEntry {
    pub id: i64,
    pub path: String,
}

impl QueueEntry {
    pub fn new(id: i64, path: String) -> QueueEntry {
        QueueEntry {
            id: id,
            path: path,
        }
    }
}
