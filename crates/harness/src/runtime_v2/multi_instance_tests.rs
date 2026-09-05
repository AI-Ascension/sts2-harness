// SPDX-License-Identifier: MIT

#[cfg(test)]
mod multi_instance_tests {
    use super::*;
    use crate::identity::IdentityError;

    fn binding(index: u64) -> Result<RuntimeV2InstanceBinding, Box<dyn std::error::Error>> {
        let context = RuntimeV2Context::new(
            format!("instance-{index}"),
            format!("gateway-session-{index}"),
            format!("lease-{index}"),
            1,
        )?;
        let run_id = RunId::new(index).ok_or(IdentityError::ZeroValue)?;
        let episode_id = EpisodeId::new(index).ok_or(IdentityError::ZeroValue)?;
        let trajectory_id = TrajectoryId::new(index).ok_or(IdentityError::ZeroValue)?;
        let trace_id = TraceId::new(index).ok_or(IdentityError::ZeroValue)?;
        let artifact_id = ArtifactId::new(index).ok_or(IdentityError::ZeroValue)?;
        Ok(RuntimeV2InstanceBinding::new(
            context,
            format!("mcp-session-{index}"),
            "harness",
            18_000 + u16::try_from(index)?,
            run_id,
            episode_id,
            trajectory_id,
            trace_id,
            artifact_id,
        )?)
    }

    fn item(
        binding: &RuntimeV2InstanceBinding,
        index: u64,
    ) -> Result<RuntimeV2WorkItem, Box<dyn std::error::Error>> {
        let request_id = RequestId::new(index).ok_or(IdentityError::ZeroValue)?;
        Ok(RuntimeV2WorkItem::new(
            binding,
            request_id,
            RuntimeV2OperationId::new(format!("op-{}-{index}", binding.instance_id()))?,
            RuntimeV2Action::end_turn(),
        )?)
    }

    fn take_next(
        coordinator: &mut RuntimeV2Coordinator,
    ) -> Result<RuntimeV2WorkItem, RuntimeV2CoordinatorError> {
        coordinator
            .next_work()
            .ok_or(RuntimeV2CoordinatorError::UnknownOperation)
    }

    #[test]
    fn registers_four_isolated_lineages_and_rejects_namespace_reuse()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = RuntimeV2CoordinatorConfig::new(4, 8, 4)?;
        let mut coordinator = RuntimeV2Coordinator::new(config);
        coordinator.register_instance(binding(1)?)?;
        assert_eq!(
            coordinator.register_instance(binding(1)?),
            Err(RuntimeV2CoordinatorError::DuplicateInstance)
        );
        let mut conflicting = binding(6)?;
        conflicting.mcp_session_id = String::from("mcp-session-1");
        assert_eq!(
            coordinator.register_instance(conflicting),
            Err(RuntimeV2CoordinatorError::NamespaceConflict)
        );
        for index in 2..=4 {
            coordinator.register_instance(binding(index)?)?;
        }
        assert_eq!(coordinator.snapshot().instances.len(), 4);
        assert_eq!(
            coordinator.register_instance(binding(5)?),
            Err(RuntimeV2CoordinatorError::InstanceLimit)
        );
        Ok(())
    }

    #[test]
    fn round_robin_is_fair_and_serial_per_instance() -> Result<(), Box<dyn std::error::Error>> {
        let config = RuntimeV2CoordinatorConfig::new(4, 8, 4)?;
        let mut coordinator = RuntimeV2Coordinator::new(config);
        let first = binding(1)?;
        let second = binding(2)?;
        coordinator.register_instance(first.clone())?;
        coordinator.register_instance(second.clone())?;
        coordinator.admit(item(&first, 1)?)?;
        coordinator.admit(item(&first, 2)?)?;
        coordinator.admit(item(&second, 3)?)?;

        let first_item = take_next(&mut coordinator)?;
        assert_eq!(first_item.binding().instance_id(), "instance-1");
        assert_eq!(coordinator.active_len(), 1);
        let second_item = take_next(&mut coordinator)?;
        assert_eq!(second_item.binding().instance_id(), "instance-2");
        assert!(coordinator.next_work().is_none());
        coordinator.complete(second_item.operation_id(), RuntimeV2Status::Settled)?;
        coordinator.complete(first_item.operation_id(), RuntimeV2Status::Settled)?;

        let final_item = take_next(&mut coordinator)?;
        assert_eq!(final_item.operation_id().as_str(), "op-instance-1-2");
        coordinator.complete(final_item.operation_id(), RuntimeV2Status::Settled)?;
        assert_eq!(coordinator.snapshot().completed, 3);
        assert_eq!(coordinator.snapshot().retained, 3);
        assert_eq!(
            coordinator.admit(RuntimeV2WorkItem::new(
                &second,
                RequestId::new(8).ok_or(IdentityError::ZeroValue)?,
                first_item.operation_id().clone(),
                RuntimeV2Action::end_turn(),
            )?),
            Err(RuntimeV2CoordinatorError::OperationRetained)
        );
        Ok(())
    }

    #[test]
    fn global_and_instance_capacity_never_forward_overload()
    -> Result<(), Box<dyn std::error::Error>> {
        let per_instance_config = RuntimeV2CoordinatorConfig::new(2, 4, 1)?;
        let mut per_instance = RuntimeV2Coordinator::new(per_instance_config);
        let first = binding(1)?;
        let second = binding(2)?;
        per_instance.register_instance(first.clone())?;
        per_instance.register_instance(second.clone())?;
        let first_item = item(&first, 1)?;
        per_instance.admit(first_item.clone())?;
        assert_eq!(
            per_instance.admit(item(&first, 2)?),
            Err(RuntimeV2CoordinatorError::InstanceQueueFull)
        );

        let global_config = RuntimeV2CoordinatorConfig::new(2, 2, 2)?;
        let mut coordinator = RuntimeV2Coordinator::new(global_config);
        coordinator.register_instance(first.clone())?;
        coordinator.register_instance(second.clone())?;
        coordinator.admit(first_item.clone())?;
        assert_eq!(coordinator.admit(item(&first, 2)?), Ok(2));
        assert_eq!(
            coordinator.admit(item(&second, 3)?),
            Err(RuntimeV2CoordinatorError::GlobalQueueFull)
        );
        assert_eq!(
            coordinator.admit(item(&second, 4)?),
            Err(RuntimeV2CoordinatorError::GlobalQueueFull)
        );
        assert_eq!(
            coordinator.admit(RuntimeV2WorkItem::new(
                &second,
                RequestId::new(5).ok_or(IdentityError::ZeroValue)?,
                first_item.operation_id().clone(),
                RuntimeV2Action::end_turn(),
            )?),
            Err(RuntimeV2CoordinatorError::OperationInFlight)
        );
        assert_eq!(coordinator.snapshot().queued, 2);
        Ok(())
    }

    #[test]
    fn unknown_holds_lane_until_reconciled_without_blocking_other_instances()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut coordinator = RuntimeV2Coordinator::new(RuntimeV2CoordinatorConfig::new(2, 4, 4)?);
        let first = binding(1)?;
        let second = binding(2)?;
        coordinator.register_instance(first.clone())?;
        coordinator.register_instance(second.clone())?;
        coordinator.admit(item(&first, 1)?)?;
        coordinator.admit(item(&first, 2)?)?;
        let active = take_next(&mut coordinator)?;
        for _ in 0..2 {
            coordinator.complete(active.operation_id(), RuntimeV2Status::Unknown)?;
        }
        assert!(coordinator.next_work().is_none());
        assert_eq!(coordinator.snapshot().unknown, 1);
        assert_eq!(coordinator.snapshot().completed, 0);
        coordinator.admit(item(&second, 3)?)?;
        let other = take_next(&mut coordinator)?;
        assert_eq!(other.binding().instance_id(), second.instance_id());
        coordinator.complete(active.operation_id(), RuntimeV2Status::Settled)?;
        let next = take_next(&mut coordinator)?;
        assert_eq!(next.operation_id().as_str(), "op-instance-1-2");
        assert_eq!(coordinator.snapshot().completed, 1);
        Ok(())
    }

    #[test]
    fn queued_cancel_and_shutdown_leave_active_work_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = RuntimeV2CoordinatorConfig::new(2, 4, 4)?;
        let mut coordinator = RuntimeV2Coordinator::new(config);
        let first = binding(1)?;
        coordinator.register_instance(first.clone())?;
        let active = item(&first, 1)?;
        let queued = item(&first, 2)?;
        coordinator.admit(active.clone())?;
        coordinator.admit(queued.clone())?;
        assert_eq!(coordinator.cancel_queued(queued.operation_id())?, queued);
        let dispatched = take_next(&mut coordinator)?;
        let report = coordinator.shutdown();
        assert!(report.cancelled().is_empty());
        assert_eq!(
            report.active_operations(),
            &[dispatched.operation_id().clone()]
        );
        assert_eq!(
            coordinator.admit(active),
            Err(RuntimeV2CoordinatorError::AdmissionClosed)
        );
        coordinator.complete(dispatched.operation_id(), RuntimeV2Status::Unknown)?;
        assert_eq!(coordinator.snapshot().cancelled, 1);
        assert_eq!(coordinator.snapshot().completed, 0);
        assert_eq!(coordinator.snapshot().active, 1);
        assert_eq!(coordinator.snapshot().unknown, 1);
        Ok(())
    }

    #[test]
    fn unknown_and_service_time_metrics_are_bounded_per_instance()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = RuntimeV2CoordinatorConfig::new(2, 4, 4)?;
        let mut coordinator = RuntimeV2Coordinator::new(config);
        let first = binding(1)?;
        let second = binding(2)?;
        coordinator.register_instance(first.clone())?;
        coordinator.register_instance(second.clone())?;
        coordinator.admit(item(&first, 1)?)?;
        coordinator.admit(item(&second, 2)?)?;

        let first_work = take_next(&mut coordinator)?;
        coordinator.complete_with_service_time(
            first_work.operation_id(),
            RuntimeV2Status::Settled,
            17,
        )?;
        let second_work = take_next(&mut coordinator)?;
        coordinator.complete_with_service_time(
            second_work.operation_id(),
            RuntimeV2Status::Unknown,
            23,
        )?;
        assert_eq!(coordinator.snapshot().service_time_samples, 1);
        coordinator.complete_with_service_time(
            second_work.operation_id(),
            RuntimeV2Status::Settled,
            23,
        )?;
        coordinator.admit(item(&first, 3)?)?;
        let rejected_work = take_next(&mut coordinator)?;
        coordinator.complete(rejected_work.operation_id(), RuntimeV2Status::Rejected)?;
        coordinator.admit(item(&second, 4)?)?;
        let cancelled_work = take_next(&mut coordinator)?;
        coordinator.complete(cancelled_work.operation_id(), RuntimeV2Status::Cancelled)?;

        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.unknown, 1);
        assert_eq!(snapshot.rejected, 1);
        assert_eq!(snapshot.cancelled, 1);
        assert_eq!(snapshot.service_time_samples, 2);
        assert_eq!(snapshot.service_time_total_millis, 40);
        assert_eq!(snapshot.service_time_max_millis, 23);
        let first_snapshot = snapshot
            .instances
            .iter()
            .find(|instance| instance.instance_id == "instance-1")
            .ok_or("first instance snapshot missing")?;
        assert_eq!(first_snapshot.unknown, 0);
        assert_eq!(first_snapshot.rejected, 1);
        assert_eq!(first_snapshot.cancelled, 0);
        assert_eq!(first_snapshot.service_time_samples, 1);
        assert_eq!(first_snapshot.service_time_total_millis, 17);
        assert_eq!(first_snapshot.service_time_max_millis, 17);
        let second_snapshot = snapshot
            .instances
            .iter()
            .find(|instance| instance.instance_id == "instance-2")
            .ok_or("second instance snapshot missing")?;
        assert_eq!(second_snapshot.unknown, 1);
        assert_eq!(second_snapshot.rejected, 0);
        assert_eq!(second_snapshot.cancelled, 1);
        assert_eq!(second_snapshot.service_time_samples, 1);
        assert_eq!(second_snapshot.service_time_total_millis, 23);
        assert_eq!(second_snapshot.service_time_max_millis, 23);
        Ok(())
    }
}
