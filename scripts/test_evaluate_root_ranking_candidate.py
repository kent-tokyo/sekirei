from evaluate_root_ranking_candidate import evaluate


def diagnostic(ordered, loss, margin):
    return {
        "pairs_scored": 10,
        "teacher_preferred_ordered_pairs": ordered,
        "teacher_preferred_ordering_rate": ordered / 10,
        "mean_pairwise_logistic_loss": loss,
        "mean_parent_oriented_margin_cp": margin,
    }


def test_rejects_one_metric_only_improvement():
    result = evaluate(diagnostic(5, 1.0, 2.0), diagnostic(6, 1.1, 2.1), 10)
    assert result["checks"] == {
        "ordering_nonworse": True, "loss_nonworse": False, "margin_nonworse": True,
    }
    assert result["status"] == "rejected_no_cherry_picking"


def test_accepts_only_all_nonworse_metrics():
    result = evaluate(diagnostic(5, 1.0, 2.0), diagnostic(5, 1.0, 2.0), 10)
    assert result["status"] == "eligible_for_next_diagnostic_only"
