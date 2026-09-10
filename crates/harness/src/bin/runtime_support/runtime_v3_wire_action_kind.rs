// SPDX-License-Identifier: MIT

pub(super) const fn action_kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::StartRun => "start_run",
        ActionKind::SelectCharacter => "select_character",
        ActionKind::SelectMapNode => "select_map_node",
        ActionKind::PlayCard => "play_card",
        ActionKind::UsePotion => "use_potion",
        ActionKind::EndTurn => "end_turn",
        ActionKind::ChooseReward => "choose_reward",
        ActionKind::SkipReward => "skip_reward",
        ActionKind::Proceed => "proceed",
        ActionKind::ConfirmSelection => "confirm_selection",
        ActionKind::CancelSelection => "cancel_selection",
        ActionKind::SelectPlayer => "select_player",
        ActionKind::ShopPurchase => "shop_purchase",
        ActionKind::ShopRemove => "shop_remove",
        ActionKind::Rest => "rest",
        ActionKind::RestOption => "rest_option",
        ActionKind::Smith => "smith",
        ActionKind::EventChoice => "event_choice",
        ActionKind::SelectCard => "select_card",
        ActionKind::ConfirmVictory => "confirm_victory",
        ActionKind::SaveQuit => "save_quit",
    }
}
