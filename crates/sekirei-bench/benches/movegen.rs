use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sekirei_core::{
    board::Board,
    eval::evaluate,
    movegen::{generate_legal_captures, generate_legal_moves},
    nnue::NnueWeights,
    perft::perft,
    search::{SearchConfig, Searcher},
    tt::Tt,
};

fn bench_movegen(c: &mut Criterion) {
    c.bench_function("legal_moves_startpos", |b| {
        let board = Board::startpos();
        b.iter(|| {
            let mut b = board.clone();
            generate_legal_moves(black_box(&mut b))
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

criterion_group!(
    benches,
    bench_movegen,
    bench_capture_movegen,
    bench_perft3,
    bench_search_depth4,
    bench_evaluate,
    bench_nnue_evaluate,
    bench_do_undo
);
criterion_main!(benches);
