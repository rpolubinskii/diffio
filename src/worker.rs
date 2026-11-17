use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};

use crate::{
    diff::Diff,
    git::{FileEntry, Repository, Result, View},
};

#[derive(Clone, Debug)]
pub struct Request {
    pub generation: u64,
    pub view: View,
    pub selected: Option<PathBuf>,
}

pub struct Snapshot {
    pub files: Vec<FileEntry>,
    pub diff: Option<Diff>,
}

pub struct Response {
    pub request: Request,
    pub result: Result<Snapshot>,
}

pub struct Worker {
    requests: Sender<Request>,
    responses: Receiver<Response>,
    busy: bool,
}

impl Worker {
    pub fn new(repo: Repository) -> Result<Self> {
        let (requests, incoming) = mpsc::channel::<Request>();
        let (outgoing, responses) = mpsc::channel();
        thread::Builder::new()
            .name("git-refresh".into())
            .spawn(move || {
                while let Ok(request) = incoming.recv() {
                    let result = refresh(&repo, &request);
                    if outgoing.send(Response { request, result }).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("Cannot start Git refresh worker: {e}"))?;
        Ok(Self {
            requests,
            responses,
            busy: false,
        })
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn request(&mut self, request: Request) -> Result<bool> {
        if self.busy {
            return Ok(false);
        }
        self.requests
            .send(request)
            .map_err(|e| format!("Git worker stopped: {e}"))?;
        self.busy = true;
        Ok(true)
    }

    pub fn receive(&mut self) -> Result<Option<Response>> {
        match self.responses.try_recv() {
            Ok(response) => {
                self.busy = false;
                Ok(Some(response))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("Git refresh worker disconnected".into()),
        }
    }
}

pub fn refresh(repo: &Repository, request: &Request) -> Result<Snapshot> {
    let files: Vec<_> = repo
        .status()?
        .into_iter()
        .filter(|file| file.visible(request.view))
        .collect();
    let diff = request.selected.as_ref().map(|path| {
        files.iter().find(|entry| &entry.path == path)
            .map(|entry| repo.diff(entry, request.view))
            .unwrap_or_else(|| Ok(Diff::message("This file has no changes in this view. It may have been committed or restored.")))
    }).transpose()?;
    Ok(Snapshot { files, diff })
}
