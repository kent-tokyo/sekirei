# 設計記録

以下は設計時の判断と制限を残す資料です。
2026-11-16の[計画リセット](../plans/README.md)により、未完了案の継続予定はありません。

| 記録 | 内容 |
|---|---|
| [Lazy SMPの由来](lazy_smp_provenance.md) | 実装・参照元・探索方式の境界 |
| [共有TTの書込み監査](shared_tt_write_topology_audit.md) | atomic配列と公開整合性の検討 |
| [NNUE構造の検討](nnue_architecture_next_candidate.md) | B-smallの実装・検証と棋力未確認の判定 |

コードの現状は該当する実装で確認します。過去の設計案を既定動作や実施予定として扱いません。
