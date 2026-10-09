
use rkvm_net::key::Key;

use std::collections::{HashMap, HashSet};

use crate::set::Set;

#[derive(Clone, Copy, Debug)]
pub enum KeyAction {
    NextClient,
    Goto(usize),
    Forward,
    Delay,
}

#[derive(Debug)]
pub struct KeyPressed {
    pub id: usize,
    pub key: Key,
}

pub struct KeyState {
    propagate: bool,
    all_prefixes: HashSet<Set<Key>>,
    actions: HashMap<Set<Key>,KeyAction>,
    all_pressed: Set<Key>,
}

impl KeyState {
    pub fn new(propagate: bool) -> Self {
        KeyState {
            propagate: propagate,
            all_prefixes: HashSet::new(),
            actions: HashMap::new(),
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

    pub fn update(&mut self, key: &Key, down: &bool) -> KeyAction {
        match down {
            true => {
                self.all_pressed.insert(*key);
                if let Some(action) = self.actions.get(&self.all_pressed) {
                    *action
                } else if !self.propagate && self.all_prefixes.contains(&self.all_pressed) {
                    KeyAction::Delay
                } else {
                    KeyAction::Forward
                }
            }
            false => {
                self.all_pressed.remove(key);
                KeyAction::Forward
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
