from analyze_component_aa_groups import family


def test_operation_families_are_disjoint_and_predeclared():
    assert family("init_sfen_warm", "sekirei") == "initialization"
    assert family("drop_only", "sekirei_nnue_forward") == "nnue"
    assert family("move_kind_drop", "sekirei_do_undo") == "board_update"
    assert family("encode_raw_list", "sekirei") == "move_representation"
    assert family("drop_only", "sekirei_generate_vec") == "move_generation"
