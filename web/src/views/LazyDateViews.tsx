// FullCalendar and vis-timeline load when the first calendar or timeline view shows, not with
// the app: together they are the largest part of what the app would otherwise load up front.
// Each shows nothing until its code has arrived. There is no Suspense boundary: one would also
// hold back the view whenever a resource inside it loads.

import { lazy } from "solid-js";

/** A query view's calendar, once FullCalendar has loaded. */
export const CalendarView = lazy(() => import("./DateViews").then((m) => ({ default: m.CalendarView })));

/** A query view's timeline, once vis-timeline has loaded. */
export const TimelineView = lazy(() => import("./DateViews").then((m) => ({ default: m.TimelineView })));
