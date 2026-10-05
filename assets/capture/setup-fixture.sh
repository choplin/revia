#!/usr/bin/env bash
# Creates the representative repository used by the README screenshots.
# Usage: setup-fixture.sh <directory>
set -euo pipefail

dir="${1:?usage: setup-fixture.sh <directory>}"
rm -rf "$dir"
mkdir -p "$dir"
cd "$dir"

git init --quiet --template= --initial-branch=main
git config user.name "Demo"
git config user.email "demo@example.com"
git config commit.gpgsign false

mkdir -p src web docs

cat > src/lib.rs <<'RS'
//! Task tracking service.

pub mod store;

use store::{Store, Task};

/// Creates a task and returns its identifier.
pub fn create_task(store: &mut Store, title: &str) -> u64 {
    let id = store.next_id();
    store.insert(Task {
        id,
        title: title.to_string(),
        done: false,
    });
    id
}

/// Marks a task as complete.
pub fn complete_task(store: &mut Store, id: u64) -> bool {
    match store.get_mut(id) {
        Some(task) => {
            task.done = true;
            true
        }
        None => false,
    }
}

/// Lists tasks that are still open.
pub fn open_tasks(store: &Store) -> Vec<&Task> {
    store.iter().filter(|task| !task.done).collect()
}
RS

cat > src/store.rs <<'RS'
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Default)]
pub struct Store {
    tasks: BTreeMap<u64, Task>,
    last_id: u64,
}

impl Store {
    pub fn next_id(&mut self) -> u64 {
        self.last_id += 1;
        self.last_id
    }

    pub fn insert(&mut self, task: Task) {
        self.tasks.insert(task.id, task);
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }
}
RS

cat > web/api.ts <<'TS'
export interface Task {
  id: number;
  title: string;
  done: boolean;
}

const BASE_URL = "/api/tasks";

export async function listTasks(): Promise<Task[]> {
  const response = await fetch(BASE_URL);
  return response.json();
}

export async function createTask(title: string): Promise<Task> {
  const response = await fetch(BASE_URL, {
    method: "POST",
    body: JSON.stringify({ title }),
  });
  return response.json();
}
TS

cat > docs/usage.md <<'MD'
# Usage

Create a task with `create_task` and complete it with `complete_task`.
`open_tasks` returns the tasks that are still open.
MD

git add .
git commit --quiet -m "Initial task service"

# Working-tree changes reviewed in the screenshots.
cat > src/lib.rs <<'RS'
//! Task tracking service.

pub mod config;
pub mod store;

use config::Limits;
use store::{Store, Task};

/// Creates a task and returns its identifier, or `None` when the title is empty.
pub fn create_task(store: &mut Store, limits: &Limits, title: &str) -> Option<u64> {
    let title = title.trim();
    if title.is_empty() || title.len() > limits.max_title_len {
        return None;
    }
    let id = store.next_id();
    store.insert(Task {
        id,
        title: title.to_string(),
        done: false,
    });
    Some(id)
}

/// Marks a task as complete.
pub fn complete_task(store: &mut Store, id: u64) -> bool {
    store.get_mut(id).map(|task| task.done = true).is_some()
}

/// Lists tasks that are still open, oldest first.
pub fn open_tasks(store: &Store) -> Vec<&Task> {
    store.iter().filter(|task| !task.done).collect()
}
RS

cat > src/store.rs <<'RS'
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Default)]
pub struct Store {
    tasks: BTreeMap<u64, Task>,
    last_id: u64,
}

impl Store {
    pub fn next_id(&mut self) -> u64 {
        self.last_id = self.last_id.checked_add(1).expect("task id overflow");
        self.last_id
    }

    pub fn insert(&mut self, task: Task) {
        self.tasks.insert(task.id, task);
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }
}
RS

cat > src/config.rs <<'RS'
/// Validation limits applied to incoming tasks.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_title_len: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_title_len: 200 }
    }
}
RS

cat > web/api.ts <<'TS'
export interface Task {
  id: number;
  title: string;
  done: boolean;
}

const BASE_URL = "/api/v1/tasks";

export async function listTasks(): Promise<Task[]> {
  const response = await fetch(BASE_URL);
  if (!response.ok) {
    throw new Error(`Failed to list tasks: ${response.status}`);
  }
  return response.json();
}

export async function createTask(title: string): Promise<Task> {
  const response = await fetch(BASE_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ title }),
  });
  return response.json();
}
TS

git mv docs/usage.md docs/guide.md
cat > docs/guide.md <<'MD'
# Guide

Create a task with `create_task` and complete it with `complete_task`.
`open_tasks` returns the tasks that are still open.
MD

echo "__FIXTURE_READY__"
