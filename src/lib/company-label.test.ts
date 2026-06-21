import { describe, it, expect } from "vitest";
import { friendlyCompanyLabel, type CompanyRef } from "./company-label";

const companies: CompanyRef[] = [
  { uid: "cmp_01indigo", slug: "indigo", name: "Indigo" },
  { uid: "cmp_01acme", slug: "acme" }, // no name → falls back to slug
];

describe("friendlyCompanyLabel", () => {
  it("maps the personal vault UID to Personal", () => {
    expect(friendlyCompanyLabel("prs_01abc", companies)).toBe("Personal");
  });

  it("maps null/undefined (personal) to Personal", () => {
    expect(friendlyCompanyLabel(null, companies)).toBe("Personal");
    expect(friendlyCompanyLabel(undefined, companies)).toBe("Personal");
  });

  it("resolves a known company UID to its name", () => {
    expect(friendlyCompanyLabel("cmp_01indigo", companies)).toBe("Indigo");
  });

  it("falls back to slug when a known company has no name", () => {
    expect(friendlyCompanyLabel("cmp_01acme", companies)).toBe("acme");
  });

  it("uses a clean generic for an unknown company UID", () => {
    expect(friendlyCompanyLabel("cmp_01unknown", companies)).toBe("a company");
  });

  it("passes an already-friendly slug through unchanged", () => {
    expect(friendlyCompanyLabel("indigo", companies)).toBe("indigo");
  });
});
