// SPDX-License-Identifier: MIT

pub(super) const fn stage_name(stage: sts2_harness::EpisodeStage) -> &'static str {
    match stage {
        sts2_harness::EpisodeStage::Setup => "setup",
        sts2_harness::EpisodeStage::Map => "map",
        sts2_harness::EpisodeStage::Combat => "combat",
        sts2_harness::EpisodeStage::Reward => "reward",
        sts2_harness::EpisodeStage::Shop => "shop",
        sts2_harness::EpisodeStage::Event => "event",
        sts2_harness::EpisodeStage::Rest => "rest",
        sts2_harness::EpisodeStage::Selection => "selection",
        sts2_harness::EpisodeStage::Victory => "victory",
        sts2_harness::EpisodeStage::Defeat => "defeat",
        sts2_harness::EpisodeStage::Recovery => "recovery",
        sts2_harness::EpisodeStage::Unknown => "unknown",
    }
}
