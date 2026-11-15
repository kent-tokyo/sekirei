# NM01b: suLWr の王手に対する独立応手検証

2026-11-15、結果を観測する前に固定。目的は、NM01の開発局面 `suLWr` で強制root探索が `B*4a` に返した mate score を、別の全合法応手再生で裏づけられるか調べること。対照の `6c5b` が敗着であること、`B*4a` の最短性・唯一性、探索変更の有効性はこの検査から主張しない。

- 入力: `data/runs/public-next-move-20261115/cases-rebuild.json` の `suLWr`、SFEN `l+P5nl/2+S3k2/p2+R1gspp/4ppp2/7N1/4PPP2/P+p3G1PP/3+p2K2/L7+r b BGPbg2s2nl2p 81`。この先手番局面から証明木を作り、初手が `B*4a` か確認する。出典は `https://lishogi.org/training/suLWr`、元対局ID `KyIRPX05`。掲載手は教師・正解判定に用いない。
- ソースは現在の追跡済みHEAD `baa77c1db3b8a1b8f10fad15db7aae28e6d9ae2c` を `git archive` で `/private/tmp` に固定する。共有worktreeの未コミット変更と並行Cargoには触れない。既存 `mate::prove_mate_existence` の候補抽出後に行う `verify_mate_proof_tree` を判定器とする。追加する小さな実行入口のSHAと実行ファイルSHAを記録する。NNUE重みは使用しない。
- 一度だけ、1T、探索最大500,000節点・内部期限3秒、最大25手、抽出と再生を各100,000節点、外側5秒、RSS目安512MiB、出力1MiBで実行する。ビルドは他のSekirei Cargo/rustcが終わり、空き容量とメモリを再確認してから1 job・最大180秒・出力5MiBで行う。資源が不足すれば実行しない。
- `tree=Some` かつ `stop_reason=None`、検証節点・代表手順・最長枝を保存できた場合に限り「25手以内の詰み存在を全合法応手で検証」とする。初手が `B*4a` の場合だけNM01強制rootの詰み得点を裏づけたとみなす。別初手ならその別手の存在証明に限定する。期限、節点、抽出、再生で止まれば `Unknown` とし、上限を伸ばさない。入力不一致や検証失敗は別に記録する。結果が何であれ `6c5b` との優劣や候補採用には進めず、NM01～NM11の候補不足を再順位付けする。

## 一度だけの実行結果

HEAD archiveを隔離ディレクトリへ展開し、上記の[実行入口](run_nm01b_proof.rs)を追加して `cargo build --release --offline -p sekirei-core --example run_nm01b_proof -j 1` を90秒で完了。初回の実行入口SHA-256 `0ee2fc6ce6da4ab565feea4db4806770c85fe9689be2dacb5deb58d1030f95cf`、実行ファイルSHA-256 `01f6eacb9eaa8fe454a36a9d2158892e426bafb371a0b25f173b5a9cda583da1`。共有worktreeの別Cargoが終了した後に逐次実行し、探索は外側0.16秒で終了した。rawはGit対象外の `data/runs/public-next-move-20261115/nm01b-proof.txt`、SHA-256 `e3f8dc05549a70b5a5d069f10a71a17cbdebca2590c96e457486640e353b7aa1`。

コミット前に実行入口を`rustfmt`で整形した（ロジック変更なし）。公開する入口SHA-256 `a38eda3f7d0640d215391ec37524919de07094c89393a730d0d33819585b2bea`を同じ隔離HEADで再ビルドし、実行ファイルSHA-256 `f3cd332095f0bac417da0d1c00f999fe9a1ed31247e3377d4f83ccd84c761aef`を得た。整形後の再現確認でも下記の結果・節点数・代表枝は一致した。出力 `data/runs/public-next-move-20261115/nm01b-proof-format-verify.txt` のSHA-256は初回と同じ `e3f8dc05549a70b5a5d069f10a71a17cbdebca2590c96e457486640e353b7aa1`。独立試行や追加の候補選定には数えない。

結果は `verified=true`、`invalid=None`、`stop=None`。探索7,451節点、抽出420節点、全合法応手の再生158節点。証明木の初手は `B*4a`、最長枝は19手、代表枝は `B*4a 3b4a 6c4c L*4b G*5b 4a3a G*3b`。よって、この局面に `B*4a` から19手以内の詰み手順が存在することを検証した。これは最短手数・唯一手の証明ではない。NM01の10万節点探索が返した `6c5b` も別の勝ち筋を持つ可能性があり、敗着と断定できない。

再順位付け: 単一例で `B*4a` の詰み存在が確認できても、別手の品質と同種の独立再現が欠ける。root詰み事前探索や探索定数の候補作成、未使用確認15題の消費、対局資格には進まない。NM10/NM11までの公開問題経路の停止は維持する。再開条件は、別対局でr6の実返却手に具体的な反証と合法救済手が得られるか、別手の品質を独立に固定・検証できる手段ができること。
