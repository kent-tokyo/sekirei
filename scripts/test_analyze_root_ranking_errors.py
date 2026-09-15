from analyze_root_ranking_errors import analyze


def pair(parent, category, gap, correct, margin):
    return {
        "parent_id": parent, "category": category, "teacher_score_gap_cp": gap,
        "higher_move_usi": "7g7f", "lower_move_usi": "2g2f",
        "parent_oriented_margin_cp": margin, "teacher_order_preserved": correct,
    }


def test_inconclusive_when_global_candidate_was_rejected():
    baseline = {("p", "7g7f", "2g2f"): pair("p", "opening_control", 40, False, -2)}
    candidate = {("p", "7g7f", "2g2f"): pair("p", "opening_control", 40, True, 2)}
    result = analyze(baseline, candidate, 1, False)
    assert result["overall"]["candidate_recovered"] == 1
    assert result["status"] == "inconclusive_no_one_factor_hypothesis"


def test_support_requires_predeclared_bucket_conditions():
    baseline, candidate = {}, {}
    for index in range(12):
        key = (f"p{index}", "7g7f", "2g2f")
        baseline[key] = pair(f"p{index}", "opening_control", 40, False, -10)
        candidate[key] = pair(f"p{index}", "opening_control", 40, index < 4, 10 if index < 4 else -10)
    result = analyze(baseline, candidate, 12, True)
    assert result["supported_buckets"] == ["category:opening_control", "teacher_gap_band:medium"]
    assert result["status"] == "one_factor_hypothesis_supported"
