    fn cache_test_list(board: &mut Board, attacker: bool) -> std::rc::Rc<ChildList> {
        let solver = bounds_test_solver(25);
        let moves = solver.children(board, attacker);
        let hashes = moves.iter().map(|&mv| {
            let undo = board.do_move_for_search(mv);
            let hash = board.hash();
            board.undo_move_for_search(undo);
            hash
        }).collect();
        std::rc::Rc::new(ChildList { moves, hashes })
    }

    #[test]
    fn child_cache_preserves_order_hashes_and_position_across_ply() {
        let sfen = "5p1nl/6S1k/5R1sp/8+b/9/9/9/9/9 b GSLPrb3gs3n2l15p 1";
        let mut board = Board::from_sfen(sfen).unwrap();
        let identity = board.position_identity();
        let before_acc = board.acc.clone();
        let before_hash = board.hash();
        let mut cache = ChildCache::default();
        assert!(cache.get(&board, true).is_none());
        let first = cache_test_list(&mut board, true);
        cache.insert(&board, true, std::rc::Rc::clone(&first));
        let hit = cache.get(&board, true).unwrap();
        let fresh = cache_test_list(&mut board, true);
        assert_eq!(hit.moves, fresh.moves);
        assert_eq!(hit.hashes, fresh.hashes);
        assert!(cache.get(&board, false).is_none());
        assert_eq!(board.position_identity(), identity);
        assert_eq!(board.hash(), before_hash);
        assert_eq!(board.acc, before_acc);
        assert_eq!(crate::sfen::board_to_sfen(&board), sfen);
        let later = Board::from_sfen(&format!("{}99", sfen.trim_end_matches('1'))).unwrap();
        assert_eq!(cache.get(&later, true).unwrap().moves, first.moves);
    }

    #[test]
    fn child_cache_rejects_wrong_identity_and_respects_both_caps() {
        let mut board = Board::from_sfen(SHORTEST_THREE_SFEN).unwrap();
        let list = cache_test_list(&mut board, true);
        assert!(!list.moves.is_empty());
        let other = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();
        let mut cache = ChildCache::default();
        cache.entries.insert((board.hash(), true), CachedChildren {
            identity: other.position_identity(), list: std::rc::Rc::clone(&list),
        });
        assert!(cache.get(&board, true).is_none());
        let mut no_entries = ChildCache { entry_limit: 0, ..ChildCache::default() };
        no_entries.insert(&board, true, std::rc::Rc::clone(&list));
        assert!(no_entries.entries.is_empty());
        let mut too_many_children = ChildCache { child_limit: list.moves.len() - 1, ..ChildCache::default() };
        too_many_children.insert(&board, true, std::rc::Rc::clone(&list));
        assert_eq!(too_many_children.children, 0);
        let mut exact = ChildCache { entry_limit: 1, child_limit: list.moves.len(), ..ChildCache::default() };
        exact.insert(&board, true, std::rc::Rc::clone(&list));
        exact.insert(&other, true, list);
        assert_eq!(exact.entries.len(), 1);
        assert!(exact.children <= exact.child_limit);
    }

