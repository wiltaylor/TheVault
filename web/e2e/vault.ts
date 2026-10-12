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

/** A board has a canvas of its own, placed on its template, and belongs to a crew. */
export const BOARD_TYPE = `label  = "Board"
plural = "Boards"

field board {
  kind  = "canvas"
  label = "Board"
}

field crew {
  kind   = "reference"
  label  = "Crew"
  target = "crew"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    slot "board"
    slot "body"
  }
}
`;

/**
 * A crew's template holds one fixed canvas: a text node, and a query node listing the crew's own
 * boards through `this`.
 */
export const CREW_TYPE = `label  = "Crew"
plural = "Crews"

template default {
  tab "Main" {
    slot "title"

    canvas {
      viewport {
        x    = 0.0
        y    = 0.0
        zoom = 1.0
      }

      node intro {
        kind   = "text"
        x      = 20.0
        y      = 20.0
        width  = 240.0
        height = 100.0
        text   = "This crew's **boards**"
      }

      node boards {
        kind   = "query"
        x      = 320.0
        y      = 20.0
        width  = 320.0
        height = 400.0
        view   = "list"
        query  = "from board where crew == this sort title asc"
      }

      edge intro_to_boards {
        from = "intro"
        to   = "boards"
      }
    }
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
