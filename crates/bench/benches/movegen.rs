use criterion::{black_box, criterion_group, criterion_main, Criterion};
use shogi_core::{
    board::Board,
    eval::evaluate,
    movegen::generate_legal_moves,
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
            let mut board  = Board::startpos();
            let searcher   = Searcher::new(Tt::new(16));
            searcher.search(black_box(&mut board), SearchConfig { max_depth: 4, time_limit: None })
        });
    });
}

fn bench_evaluate(c: &mut Criterion) {
    c.bench_function("evaluate_startpos", |b| {
        let board = Board::startpos();
        b.iter(|| evaluate(black_box(&board)));
    });
}

criterion_group!(benches, bench_movegen, bench_perft3, bench_search_depth4, bench_evaluate);
criterion_main!(benches);
