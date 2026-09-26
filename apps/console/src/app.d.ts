import type { SessionUser } from '$lib/server/vkdg/client';

declare global {
  namespace App {
    interface Locals {
      user: SessionUser | null;
    }
  }
}

export {};
