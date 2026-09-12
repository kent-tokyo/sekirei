use criterion::{Criterion, criterion_group, criterion_main};
use sekirei_core::{
    board::Board,
    eval::evaluate,
    mcts::{
        MaterialValue, MctsConfig, RootMcts, SharedTreeMcts, SharedTreeMctsConfig, TreeMcts,
        TreeMctsConfig, UniformPolicy,
    },
    movegen::{generate_legal_captures, generate_legal_moves, generate_legal_moves_into},
    nnue::NnueWeights,
    perft::perft,
    policy::top_n,
    search::{SearchConfig, Searcher, SpeculativeSearcher},
    tt::Tt,
};
use std::hint::black_box;

fn bench_movegen(c: &mut Criterion) {
    c.bench_function("legal_moves_startpos", |b| {
        let board = Board::startpos();
        b.iter(|| {
            let mut b = board.clone();
            generate_legal_moves(black_box(&mut b))
        });
    });
}

fn bench_movegen_vec_capacity(c: &mut Criterion) {
    let board = Board::startpos();
    c.bench_function("legal_moves_startpos_cold_vec", |b| {
        b.iter(|| {
            let mut board = board.clone();
            let mut moves = Vec::new();
            generate_legal_moves_into(black_box(&mut board), &mut moves);
            black_box(moves)
        });
    });
    let mut board = board;
    let mut moves = Vec::with_capacity(128);
    c.bench_function("legal_moves_startpos_warm_vec", |b| {
        b.iter(|| {
            generate_legal_moves_into(black_box(&mut board), &mut moves);
            black_box(moves.len())
        });
    });
}

fn bench_capture_movegen(c: &mut Criterion) {
    let board = Board::from_sfen("k8/9/9/4p4/4p4/9/4R4/9/8K b - 1").unwrap();
    c.bench_function("legal_captures_tactical", |b| {
        b.iter(|| {
            let mut b = board.clone();
            generate_legal_captures(black_box(&mut b))
        });
    });
}

fn bench_perft3(c: &mut Criterion) {
    c.bench_function("perft3_startpos", |b| {
        let board = Board::startpos();
        b.iter(|| {
            let mut b = board.clone();
            perft(black_box(&mut b), 3)
        });
    });
}

fn bench_search_depth4(c: &mut Criterion) {
    c.bench_function("search_depth4_startpos", |b| {
        b.iter(|| {
            let mut board = Board::startpos();
            let searcher = Searcher::new(Tt::new(16));
            searcher.search(
                black_box(&mut board),
                SearchConfig {
                    max_depth: 4,
                    time_limit: None,
                    node_limit: None,
                    soft_limit: None,
                    multi_pv: 1,
                },
            )
        });
    });
}

fn bench_evaluate(c: &mut Criterion) {
    c.bench_function("evaluate_startpos", |b| {
        let board = Board::startpos();
        b.iter(|| evaluate(black_box(&board)));
    });
}

fn bench_policy_top_n(c: &mut Criterion) {
    let board = Board::startpos();
    let tt = Tt::new(16);
    c.bench_function("policy_top_n_startpos_n2", |b| {
        b.iter(|| black_box(top_n(black_box(&board), black_box(&tt), 2)));
    });
}

fn bench_nnue_evaluate(c: &mut Criterion) {
    let board = Board::startpos();
    let weights = NnueWeights::default_lcg();
    c.bench_function("nnue_evaluate_startpos", |b| {
        b.iter(|| {
            black_box(
                board
                    .acc
                    .evaluate_with(black_box(&weights), black_box(board.side_to_move)),
            )
        });
    });
}

fn bench_nnue_evaluate_corpus(c: &mut Criterion) {
    let boards = [
        Board::startpos(),
        Board::from_sfen("9/9/9/9/4R4/9/9/9/4k4 w - 1").unwrap(),
        Board::from_sfen("4k4/9/9/9/9/9/4+P4/9/4K4 w - 2").unwrap(),
        Board::from_sfen("4k4/9/9/4R4/9/9/9/9/4K4 w - 1").unwrap(),
    ];
    let weights = NnueWeights::default_lcg();
    c.bench_function("nnue_evaluate_corpus", |b| {
        b.iter(|| {
            let mut total = 0i32;
            for board in &boards {
                total += board
                    .acc
                    .evaluate_with(black_box(&weights), black_box(board.side_to_move));
            }
            black_box(total)
        });
    });
}

fn bench_do_undo(c: &mut Criterion) {
    let mut board = Board::startpos();
    let mv = generate_legal_moves(&mut board)[0];
    c.bench_function("do_undo_startpos_quiet", |b| {
        b.iter(|| {
            let token = board.do_move(black_box(mv));
            board.undo_move(token);
            black_box(board.hash())
        });
    });
}

fn bench_tree_mcts(c: &mut Criterion) {
    let board = Board::startpos();
    let mcts = TreeMcts::default();
    let policy = UniformPolicy;
    let value = MaterialValue;
    c.bench_function("tree_mcts_256_simulations_depth4", |b| {
        b.iter(|| {
            black_box(mcts.search(
                black_box(&board),
                TreeMctsConfig {
                    simulations: 256,
                    max_depth: 4,
                    value_cache: false,
                },
                &policy,
                &value,
            ))
        });
    });
}

fn bench_root_mcts(c: &mut Criterion) {
    let board = Board::startpos();
    let mcts = RootMcts::default();
    let policy = UniformPolicy;
    let value = MaterialValue;
    c.bench_function("root_mcts_256_simulations", |b| {
        b.iter(|| {
            black_box(mcts.search(
                black_box(&board),
                MctsConfig {
                    simulations: 256,
                    ..MctsConfig::default()
                },
                &policy,
                &value,
            ))
        });
    });
}

fn bench_shared_tree_mcts(c: &mut Criterion) {
    let board = Board::startpos();
    let mcts = SharedTreeMcts::default();
    let policy = UniformPolicy;
    let value = MaterialValue;
    c.bench_function("shared_tree_mcts_256_simulations_depth4", |b| {
        b.iter(|| {
            black_box(mcts.search(
                black_box(&board),
                SharedTreeMctsConfig {
                    simulations: 256,
                    max_depth: 4,
                    ..SharedTreeMctsConfig::default()
                },
                &policy,
                &value,
            ))
        });
    });
}

fn bench_speculative_search(c: &mut Criterion) {
    let searcher = SpeculativeSearcher::new(Tt::new(16), 2);
    c.bench_function("speculative_search_depth4_startpos", |b| {
        b.iter(|| {
            searcher.reset_abort_flag();
            let mut board = Board::startpos();
            black_box(searcher.search(
                black_box(&mut board),
                SearchConfig {
                    max_depth: 4,
                    time_limit: None,
                    node_limit: None,
                    soft_limit: None,
                    multi_pv: 1,
                },
            ))
        });
    });
}

criterion_group!(
    benches,
    bench_movegen,
    bench_movegen_vec_capacity,
    bench_capture_movegen,
    bench_perft3,
    bench_search_depth4,
    bench_evaluate,
    bench_policy_top_n,
    bench_nnue_evaluate,
    bench_nnue_evaluate_corpus,
    bench_do_undo,
    bench_tree_mcts,
    bench_root_mcts,
    bench_shared_tree_mcts,
    bench_speculative_search
);
criterion_main!(benches);
