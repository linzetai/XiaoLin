import { create } from "zustand";
import * as transport from "../transport";

export interface AutomationStoreState {
  jobs: transport.CronJob[];
  loading: boolean;
  error: string | null;
  selectedJobId: string | null;
  runs: transport.CronJobRun[];

  loadJobs: () => Promise<void>;
  createJob: (
    job: Partial<transport.CronJob> & {
      name: string;
      schedule: string;
      action: transport.CronJobAction;
    },
  ) => Promise<transport.CronJob | null>;
  updateJob: (jobId: string, patch: Partial<transport.CronJob>) => Promise<void>;
  deleteJob: (jobId: string) => Promise<void>;
  runNow: (jobId: string) => Promise<boolean>;
  fetchRuns: (jobId: string) => Promise<void>;
  selectJob: (jobId: string | null) => void;
}

export const useAutomationStore = create<AutomationStoreState>((set) => ({
  jobs: [],
  loading: false,
  error: null,
  selectedJobId: null,
  runs: [],

  loadJobs: async () => {
    set({ loading: true, error: null });
    try {
      const jobs = await transport.automationsList();
      set({ jobs, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  createJob: async (job) => {
    try {
      const created = await transport.automationsCreate(job);
      if (created) {
        set((s) => ({ jobs: [...s.jobs, created] }));
      }
      return created;
    } catch (e) {
      set({ error: String(e) });
      return null;
    }
  },

  updateJob: async (jobId, patch) => {
    try {
      const updated = await transport.automationsUpdate(jobId, patch);
      if (updated) {
        set((s) => ({
          jobs: s.jobs.map((j) => (j.id === jobId ? updated : j)),
        }));
      }
    } catch (e) {
      set({ error: String(e) });
    }
  },

  deleteJob: async (jobId) => {
    try {
      await transport.automationsDelete(jobId);
      set((s) => ({
        jobs: s.jobs.filter((j) => j.id !== jobId),
        selectedJobId: s.selectedJobId === jobId ? null : s.selectedJobId,
      }));
    } catch (e) {
      set({ error: String(e) });
    }
  },

  runNow: async (jobId) => {
    try {
      return await transport.automationsRunNow(jobId);
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  fetchRuns: async (jobId) => {
    try {
      const runs = await transport.automationsRuns(jobId);
      set({ runs, selectedJobId: jobId });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  selectJob: (jobId) => set({ selectedJobId: jobId }),
}));

let _unsub: (() => void) | undefined;

export function initAutomationListener(): void {
  _unsub?.();
  _unsub = transport.onAutomationsChanged((evt) => {
    const store = useAutomationStore.getState();
    switch (evt.event) {
      case "created":
        if (evt.job) {
          useAutomationStore.setState((s) => ({
            jobs: [...s.jobs.filter((j) => j.id !== evt.jobId), evt.job!],
          }));
        }
        break;
      case "updated":
        if (evt.job) {
          useAutomationStore.setState((s) => ({
            jobs: s.jobs.map((j) => (j.id === evt.jobId ? evt.job! : j)),
          }));
        }
        break;
      case "deleted":
        useAutomationStore.setState((s) => ({
          jobs: s.jobs.filter((j) => j.id !== evt.jobId),
          selectedJobId:
            s.selectedJobId === evt.jobId ? null : s.selectedJobId,
        }));
        break;
      case "run_completed":
        if (store.selectedJobId === evt.jobId) {
          store.fetchRuns(evt.jobId);
        }
        if (evt.job) {
          useAutomationStore.setState((s) => ({
            jobs: s.jobs.map((j) => (j.id === evt.jobId ? evt.job! : j)),
          }));
        }
        break;
    }
  });
}

export function teardownAutomationListener(): void {
  _unsub?.();
  _unsub = undefined;
}
