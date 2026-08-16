import { describe, expect, it } from "vitest";
import { isAppError, toAppError, type AppError } from "./types";

describe("isAppError", () => {
  it("accepts valid AppError objects with only required fields", () => {
    const validMinimal: AppError = {
      code: "FILE_NOT_FOUND",
      title: "File Not Found",
      message: "The requested file could not be found.",
    };
    expect(isAppError(validMinimal)).toBe(true);
  });

  it("accepts valid AppError objects with string details and suggestion", () => {
    const validFull: AppError = {
      code: "INVALID_PDF",
      title: "Corrupt PDF",
      message: "The PDF header is invalid.",
      details: "qpdf exited with code 2",
      suggestion: "Try opening the file in a PDF repair tool.",
    };
    expect(isAppError(validFull)).toBe(true);
  });

  it("accepts valid AppError objects with null or undefined details and suggestion", () => {
    const withNulls: AppError = {
      code: "PERMISSION_DENIED",
      title: "Access Denied",
      message: "Cannot write to destination.",
      details: null,
      suggestion: null,
    };
    const withUndefined: AppError = {
      code: "PERMISSION_DENIED",
      title: "Access Denied",
      message: "Cannot write to destination.",
      details: undefined,
      suggestion: undefined,
    };
    expect(isAppError(withNulls)).toBe(true);
    expect(isAppError(withUndefined)).toBe(true);
  });

  it("rejects non-object and null values", () => {
    expect(isAppError(null)).toBe(false);
    expect(isAppError(undefined)).toBe(false);
    expect(isAppError("error string")).toBe(false);
    expect(isAppError(12345)).toBe(false);
    expect(isAppError(true)).toBe(false);
    expect(isAppError(Symbol("err"))).toBe(false);
    expect(isAppError(() => {})).toBe(false);
  });

  it("rejects objects missing required fields", () => {
    expect(isAppError({ title: "Error", message: "Failed" })).toBe(false);
    expect(isAppError({ code: "ERR", message: "Failed" })).toBe(false);
    expect(isAppError({ code: "ERR", title: "Error" })).toBe(false);
    expect(isAppError({})).toBe(false);
  });

  it("rejects objects with non-string code, title, or message", () => {
    expect(isAppError({ code: 500, title: "Error", message: "Failed" })).toBe(false);
    expect(isAppError({ code: "ERR", title: 404, message: "Failed" })).toBe(false);
    expect(isAppError({ code: "ERR", title: "Error", message: null })).toBe(false);
    expect(isAppError({ code: "ERR", title: "Error", message: ["Failed"] })).toBe(false);
    expect(isAppError({ code: null, title: "Error", message: "Failed" })).toBe(false);
    expect(isAppError({ code: "ERR", title: {}, message: "Failed" })).toBe(false);
  });

  it("rejects objects with invalid details or suggestion types", () => {
    const base = { code: "ERR", title: "Error", message: "Failed" };
    expect(isAppError({ ...base, details: 123 })).toBe(false);
    expect(isAppError({ ...base, details: true })).toBe(false);
    expect(isAppError({ ...base, details: ["detail"] })).toBe(false);
    expect(isAppError({ ...base, details: { nested: "error" } })).toBe(false);

    expect(isAppError({ ...base, suggestion: 456 })).toBe(false);
    expect(isAppError({ ...base, suggestion: false })).toBe(false);
    expect(isAppError({ ...base, suggestion: ["suggestion"] })).toBe(false);
    expect(isAppError({ ...base, suggestion: { tip: "help" } })).toBe(false);
  });
});

describe("toAppError", () => {
  it("passes valid AppError objects through unchanged", () => {
    const error: AppError = {
      code: "DISK_FULL",
      title: "Disk Full",
      message: "Not enough space available.",
      details: "Required: 100MB, Available: 10MB",
      suggestion: "Free up disk space and retry.",
    };
    expect(toAppError(error)).toBe(error);
  });

  it("converts standard Error instances into an UNKNOWN AppError", () => {
    const jsError = new Error("Network timeout");
    const appError = toAppError(jsError);
    expect(appError.code).toBe("UNKNOWN");
    expect(appError.title).toBe("Something went wrong");
    expect(appError.message).toBe("Network timeout");
    expect(appError.details).toBe(jsError.stack);
  });

  it("converts malformed objects into UNKNOWN AppError safely", () => {
    const malformed = { code: 123, title: null, message: false };
    const appError = toAppError(malformed);
    expect(appError.code).toBe("UNKNOWN");
    expect(appError.title).toBe("Something went wrong");
    expect(appError.message).toBe("An unexpected error occurred.");
    expect(appError.details).toBe(JSON.stringify(malformed));
  });

  it("converts string values into UNKNOWN AppError", () => {
    const appError = toAppError("Custom failure message");
    expect(appError.code).toBe("UNKNOWN");
    expect(appError.title).toBe("Something went wrong");
    expect(appError.message).toBe("Custom failure message");
    expect(appError.details).toBe(JSON.stringify("Custom failure message"));
  });
});
