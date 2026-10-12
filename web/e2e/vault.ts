// What the S5 vault holds before the server starts.

export const TASK_TYPE = `label  = "Task"
plural = "Tasks"

field status {
  kind    = "select"
  label   = "Status"
  options = ["todo", "doing", "done"]
}

field estimate {
  kind  = "int"
  label = "Estimate"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Overview" {
    slot "title"
    slot "status"
    slot "body"
  }

  tab "Details" {
    slot "estimate" {
      label = "Hours"
    }
  }
}
`;

export const SKETCH_TYPE = `label  = "Sketch"
plural = "Sketches"

field drawing {
  kind  = "ink"
  label = "Drawing"
}

template default {
  tab "Main" {
    slot "title"
  }

  tab "Drawing" {
    slot "drawing"
  }
}
`;

export const BULK_TYPE = `label  = "Bulk note"
plural = "Bulk notes"

template default {
  tab "Main" {
    slot "title"
  }
}
`;

export const BULK_NOTES = 10_000;

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** The ULID of bulk note `i`: a fixed time part and `i` in base 32. */
export function bulkId(i: number): string {
  let rest = "";
  for (let n = i; rest.length < 16; n = Math.floor(n / 32)) rest = CROCKFORD[n % 32] + rest;
  return `01JB8Z4K2M${rest}`;
}

export function bulkTitle(i: number): string {
  return `Bulk ${String(i).padStart(5, "0")}`;
}
