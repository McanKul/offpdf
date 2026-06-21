/**
 * Recent jobs list, persisted to localStorage. Stores ONLY non-sensitive UI
 * metadata (tool, label, status, output paths, timestamp) — never PDF content,
 * thumbnails, or document metadata.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { RecentJob } from "@/lib/types";

const MAX_RECENT = 25;

interface JobsState {
  recentJobs: RecentJob[];
  addJob: (job: RecentJob) => void;
  removeJob: (id: string) => void;
  clearJobs: () => void;
}

export const useJobsStore = create<JobsState>()(
  persist(
    (set) => ({
      recentJobs: [],
      addJob: (job) =>
        set((state) => ({
          recentJobs: [job, ...state.recentJobs.filter((j) => j.id !== job.id)].slice(
            0,
            MAX_RECENT,
          ),
        })),
      removeJob: (id) =>
        set((state) => ({
          recentJobs: state.recentJobs.filter((j) => j.id !== id),
        })),
      clearJobs: () => set({ recentJobs: [] }),
    }),
    {
      name: "offpdf.jobs",
      version: 1,
    },
  ),
);
