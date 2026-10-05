import { DurableObject } from "cloudflare:workers";
import type { Env } from "./env";
import { utcDay } from "./token";

/**
 * One instance per quota key. Counts live in memory only (PROTOCOL.md §9.1):
 * an evicted instance forgets, which costs at most one extra day's allowance.
 */
export class Quota extends DurableObject<Env> {
  private day = "";
  private used = 0;

  async take(limit: number): Promise<boolean> {
    const today = utcDay();
    if (today !== this.day) {
      this.day = today;
      this.used = 0;
    }
    if (this.used >= limit) return false;
    this.used++;
    return true;
  }
}
