//! Wire captured codexdc job timing into existing command and process surfaces.
use super::*;

impl ChatWidget {
    pub(super) fn on_background_job_timers(
        &mut self,
        state: codex_app_server_protocol::CodexdcBackgroundWorkState,
    ) {
        if self.thread_id().is_none_or(|id| id.to_string() != state.thread_id) {
            return;
        }
        self.background_job_timers.update(state.background_jobs_enabled, state.job_timers);
        if let Some(cell) = self.transcript.active_cell.as_mut()
            .and_then(|cell| cell.as_any_mut().downcast_mut::<ExecCell>())
        {
            cell.set_job_timers(&self.background_job_timers);
        }
        self.refresh_background_job_labels();
        self.request_redraw();
    }

    pub(crate) fn refresh_background_job_labels(&mut self) {
        if !self.background_job_timers.enabled {
            return;
        }
        let timers: Vec<_> = self.unified_exec_processes.iter()
            .filter_map(|process| self.background_job_timers.for_process(&process.key))
            .collect();
        let running = timers.iter().any(crate::background_job_timer::JobTimer::running);
        self.bottom_pane.set_background_job_timers(timers);
        if let Some(cell) = self.transcript.active_cell.as_mut()
            .and_then(|cell| cell.as_any_mut().downcast_mut::<ExecCell>())
        {
            cell.set_job_timers(&self.background_job_timers);
            if cell.is_active() && !cell.job_timers.is_empty() {
                self.bump_active_cell_revision();
            }
        }
        if running {
            self.frame_requester.schedule_frame_in(Duration::from_secs(1));
        }
    }
}
