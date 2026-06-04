import { create } from "zustand";
import {
  listProjects,
  createProject as apiCreateProject,
  updateProject as apiUpdateProject,
  deleteProject as apiDeleteProject,
  type ProjectSummary,
} from "../transport";

interface ProjectState {
  projects: Record<string, ProjectSummary>;
  activeProjectId: string | null;

  syncProjects: () => Promise<void>;
  createProject: (rootPath: string, name?: string) => Promise<ProjectSummary | null>;
  updateProject: (
    id: string,
    patch: Partial<Pick<ProjectSummary, "name" | "color" | "pinned" | "archived">>
  ) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  setActiveProjectId: (id: string | null) => void;
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: {},
  activeProjectId: null,

  syncProjects: async () => {
    try {
      const projects = await listProjects();
      const map: Record<string, ProjectSummary> = {};
      for (const p of projects) {
        map[p.id] = p;
      }
      set({ projects: map });
    } catch (e) {
      console.warn("[project-store] sync failed:", e);
    }
  },

  createProject: async (rootPath, name) => {
    try {
      const project = await apiCreateProject(rootPath, name);
      if (project) {
        set((s) => ({
          projects: { ...s.projects, [project.id]: project },
        }));
      }
      return project;
    } catch (e) {
      console.warn("[project-store] create failed:", e);
      return null;
    }
  },

  updateProject: async (id, patch) => {
    set((s) => {
      const existing = s.projects[id];
      if (!existing) return s;
      return {
        projects: { ...s.projects, [id]: { ...existing, ...patch } },
      };
    });
    try {
      await apiUpdateProject(id, patch);
    } catch (e) {
      console.warn("[project-store] update failed:", e);
      get().syncProjects();
    }
  },

  deleteProject: async (id) => {
    set((s) => {
      const { [id]: _, ...rest } = s.projects;
      return { projects: rest };
    });
    try {
      await apiDeleteProject(id);
    } catch (e) {
      console.warn("[project-store] delete failed:", e);
      get().syncProjects();
    }
  },

  setActiveProjectId: (id) => set({ activeProjectId: id }),
}));
