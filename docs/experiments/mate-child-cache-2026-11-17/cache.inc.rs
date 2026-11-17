// Search-local, bounded cache of position-derived data only.
struct ChildList {
    moves: Vec<Move>,
    hashes: Vec<u64>,
}

struct CachedChildren {
    identity: crate::board::PositionIdentity,
    list: std::rc::Rc<ChildList>,
}

struct ChildCache {
    entries: MateMap<(u64, bool), CachedChildren>,
    children: usize,
    entry_limit: usize,
    child_limit: usize,
}

impl Default for ChildCache {
    fn default() -> Self {
        Self {
            entries: MateMap::default(),
            children: 0,
            entry_limit: 8_192,
            child_limit: 65_536,
        }
    }
}

impl ChildCache {
    fn get(&self, board: &Board, attacker: bool) -> Option<std::rc::Rc<ChildList>> {
        let entry = self.entries.get(&(board.hash(), attacker))?;
        if entry.identity != board.position_identity() {
            return None;
        }
        Some(std::rc::Rc::clone(&entry.list))
    }

    fn insert(&mut self, board: &Board, attacker: bool, list: std::rc::Rc<ChildList>) {
        let key = (board.hash(), attacker);
        if self.entries.len() >= self.entry_limit
            || list.moves.len() > self.child_limit.saturating_sub(self.children)
            || self.entries.contains_key(&key)
        {
            return;
        }
        self.children += list.moves.len();
        self.entries.insert(key, CachedChildren {
            identity: board.position_identity(),
            list,
        });
    }
}

