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

field project {
  kind   = "reference"
  label  = "Project"
  target = "project"
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

template card {
  tab "Card" {
    slot "title"
    slot "status" {
      label = "State"
    }
  }
}
`;

/** A project lists its own tasks through a query view in its template, with `this` bound to it. */
export const PROJECT_TYPE = `label  = "Project"
plural = "Projects"

template default {
  tab "Overview" {
    slot "title"

    query_view {
      title = "Tasks"
      query = "from task where project == this sort title asc"

      view table {
        kind    = "table"
        title   = "Table"
        columns = ["title", "status", "estimate"]
      }

      view cards {
        kind  = "cards"
        title = "Cards"
      }
    }
  }
}
`;

/** A page is free content, where query view blocks are written in the editor. */
export const PAGE_TYPE = `label  = "Page"
plural = "Pages"

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    slot "body"
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

/** Notes of `mass`, for query views over a very large result. */
export const MASS_TYPE = `label  = "Mass note"
plural = "Mass notes"

template default {
  tab "Main" {
    slot "title"
  }
}
`;

export const MASS_NOTES = 50_000;

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

function seededId(prefix: string, i: number): string {
  let rest = "";
  for (let n = i; rest.length < 16; n = Math.floor(n / 32)) rest = CROCKFORD[n % 32] + rest;
  return `${prefix}${rest}`;
}

/** The ULID of bulk note `i`: a fixed time part and `i` in base 32. */
export function bulkId(i: number): string {
  return seededId("01JB8Z4K2M", i);
}

/** The ULID of mass note `i`, with a time part of its own. */
export function massId(i: number): string {
  return seededId("01JB8Z4K3M", i);
}

export function bulkTitle(i: number): string {
  return `Bulk ${String(i).padStart(5, "0")}`;
}
