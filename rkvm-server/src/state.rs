
use rkvm_net::Update;
use rkvm_net::event::Event;
use rkvm_net::key::{Key, KeyEvent};

use std::collections::{HashMap, HashSet};

use crate::client::Client;
use crate::set::Set;

#[derive(Clone, Copy)]
pub enum KeyAction {
    NextClient,
    Goto(usize),
    Forward,
    Delay,
}

pub struct KeyState {
    propagate: bool,
    all_prefixes: HashSet<Set<Key>>,
    actions: HashMap<Set<Key>,KeyAction>,

    pressed: HashMap<usize,HashSet<Key>>,
    all_pressed: Set<Key>,
}

impl KeyState {
    pub fn new(propagate: bool) -> Self {
        KeyState {
            propagate: propagate,
            all_prefixes: HashSet::new(),
            actions: HashMap::new(),
            pressed: HashMap::new(),
            all_pressed: Set::new(),
        }
    }

    pub fn propagate(&self) -> bool {
        self.propagate
    }

    pub fn add_action(&mut self, keys: Set<Key>, action: KeyAction) {
        self.add_prefixes(&keys);
        self.actions.insert(keys, action);
    }

    pub fn remove_device(&mut self, id: usize) {
        if let Some(keys) = self.pressed.remove(&id) {
            for key in keys {
                self.all_pressed.remove(&key);
            }
        }
    }

    pub fn update(&mut self, id: usize, key: &Key, down: &bool) -> KeyAction {
        match down {
            true => {
                match self.pressed.get_mut(&id) {
                    Some(set) => {
                        set.insert(*key);
                    }
                    None => {
                        self.pressed.insert(id, HashSet::from([*key]));
                    }
                }
                self.all_pressed.insert(*key);
            }
            false => {
                self.all_pressed.remove(key);
                if let Some(set) = self.pressed.get_mut(&id) {
                    set.remove(key);
                }
            }
        };

        if let Some(action) = self.actions.get(&self.all_pressed) {
            *action
        } else if !self.propagate && self.all_prefixes.contains(&self.all_pressed) {
            KeyAction::Delay
        } else {
            KeyAction::Forward
        }
    }

    pub async fn send_state(&self, client: &mut Client, down: bool) {
        for (id,keys) in &self.pressed {
            for key in keys {
                let update = Update::Event{ id: *id, event: Event::Key(KeyEvent{key: *key, down: down})};
                let _ = client.send(update).await;
            }
        }
    }

    fn add_prefixes(&mut self, keys: &Set<Key>) {
        let keys: Vec<Key> = keys.iter().copied().collect();

        for len in 1..keys.len() {
            let mut combination = Vec::with_capacity(len);
            Self::add_combinations(&keys, len, 0, &mut combination, &mut self.all_prefixes);
        }
    }

    fn add_combinations( keys: &[Key], len: usize, start: usize, combination: &mut Vec<Key>, prefixes: &mut HashSet<Set<Key>>,) {
        if combination.len() == len {
            prefixes.insert(combination.iter().copied().collect());
            return;
        }

        let remaining = len - combination.len();

        for i in start..=keys.len() - remaining {
            combination.push(keys[i]);
            Self::add_combinations(
                keys,
                len,
                i + 1,
                combination,
                prefixes,
            );
            combination.pop();
        }
    }
}