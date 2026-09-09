import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';

/**
 * Binary search over a cumulative offset table.
 * Returns the largest row index `r` such that offsets[r] <= target
 * (i.e. the row that contains `target`, given offsets[r+1] is its end).
 */
function findRowForOffset(offsets: Float64Array, target: number): number {
  const lastRow = offsets.length - 2; // offsets has totalRows + 1 entries
  if (lastRow < 0) return 0;
  let lo = 0;
  let hi = lastRow;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (offsets[mid] <= target) {
      lo = mid;
    } else {
      hi = mid - 1;
    }
  }
  return lo;
}

export function useVirtualizer({
  count,
  itemHeight,
  overscan = 5,
  gridItemWidth = 0,
  gridGap = 0,
}: {
  count: number;
  itemHeight: number;
  overscan?: number;
  gridItemWidth?: number;
  gridGap?: number;
}) {
  const [containerNode, setContainerNode] = useState<HTMLDivElement | null>(null);
  const containerRef = useCallback((node: HTMLDivElement | null) => {
    setContainerNode(node);
  }, []);
  const [scrollOffset, setScrollOffset] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(() =>
    typeof window !== 'undefined' ? window.innerHeight : 1000
  );
  const [columns, setColumns] = useState(() => {
    if (gridItemWidth <= 0 || typeof window === 'undefined') return 1;
    return Math.max(1, Math.floor(window.innerWidth / (gridItemWidth + gridGap)));
  });

  useLayoutEffect(() => {
    const container = containerNode;
    if (!container) return;

    const getScrollParent = (node: HTMLElement | null): HTMLElement => {
      if (!node || node === document.body) return document.documentElement;
      const overflowY = window.getComputedStyle(node).overflowY;
      const isScrollable = overflowY !== 'visible' && overflowY !== 'hidden';
      if (isScrollable && node.scrollHeight > node.clientHeight) {
        return node;
      }
      return getScrollParent(node.parentElement);
    };

    const scrollEl = getScrollParent(container);
    const scrollTarget: EventTarget = scrollEl === document.documentElement ? window : scrollEl;

    const measure = () => {
      if (!container) return;
      const rect = container.getBoundingClientRect();
      const scrollParentRect =
        scrollEl === document.documentElement
          ? { top: 0, height: window.innerHeight }
          : scrollEl.getBoundingClientRect();

      const offsetTop = rect.top - scrollParentRect.top;
      setScrollOffset(-offsetTop);
      setViewportHeight(scrollParentRect.height);

      if (gridItemWidth > 0) {
        const clientWidth = container.clientWidth;
        const computedCols = Math.max(
          1,
          Math.floor((clientWidth + gridGap) / (gridItemWidth + gridGap))
        );
        setColumns(computedCols);
      } else {
        setColumns(1);
      }
    };

    let rafId: number | null = null;
    const scheduleMeasure = () => {
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        measure();
      });
    };

    measure();

    scrollTarget.addEventListener('scroll', scheduleMeasure, { passive: true });
    window.addEventListener('resize', scheduleMeasure, { passive: true });

    const ro = new ResizeObserver(() => scheduleMeasure());
    ro.observe(container);

    return () => {
      if (rafId !== null) cancelAnimationFrame(rafId);
      scrollTarget.removeEventListener('scroll', scheduleMeasure);
      window.removeEventListener('resize', scheduleMeasure);
      ro.disconnect();
    };
  }, [containerNode, count, itemHeight, gridItemWidth, gridGap]);

  const itemHeights = useRef(new Map<number, number>());
  const indexToElement = useRef(new Map<number, Element>());
  const elementToIndex = useRef(new WeakMap<Element, number>());
  const itemResizeObserver = useRef<ResizeObserver | null>(null);
  const [heightSnapshot, setHeightSnapshot] = useState(() => new Map<number, number>());

  const measureRafId = useRef<number | null>(null);
  const scheduleRecompute = useCallback(() => {
    if (measureRafId.current !== null) return;
    measureRafId.current = requestAnimationFrame(() => {
      measureRafId.current = null;
      setHeightSnapshot(new Map(itemHeights.current));
    });
  }, []);
  useLayoutEffect(() => {
    return () => {
      if (measureRafId.current !== null) cancelAnimationFrame(measureRafId.current);
    };
  }, []);

  const recordHeight = useCallback(
    (index: number, height: number) => {
      if (height <= 0) return;
      const prev = itemHeights.current.get(index);
      if (prev !== undefined && Math.abs(prev - height) < 0.5) return;
      itemHeights.current.set(index, height);
      scheduleRecompute();
    },
    [scheduleRecompute]
  );

  // Single shared observer for the hook's lifetime — not one per row.
  useLayoutEffect(() => {
    itemResizeObserver.current = new ResizeObserver(entries => {
      for (const entry of entries) {
        const index = elementToIndex.current.get(entry.target);
        if (index === undefined) continue;
        const height = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
        recordHeight(index, height);
      }
    });
    return () => {
      itemResizeObserver.current?.disconnect();
      itemResizeObserver.current = null;
    };
  }, [recordHeight]);

  const measureElement = useCallback(
    (el: Element | null) => {
      if (!el) return;
      const raw = el.getAttribute('data-index');
      if (raw === null) return;
      const index = Number(raw);
      if (Number.isNaN(index)) return;

      const prevEl = indexToElement.current.get(index);
      if (prevEl !== el) {
        if (prevEl) itemResizeObserver.current?.unobserve(prevEl);
        indexToElement.current.set(index, el);
        elementToIndex.current.set(el, index);
        itemResizeObserver.current?.observe(el);
      }

      recordHeight(index, el.getBoundingClientRect().height);
    },
    [recordHeight]
  );

  useLayoutEffect(() => {
    if (count !== 0) return;
    itemHeights.current.clear();
    indexToElement.current.forEach(el => itemResizeObserver.current?.unobserve(el));
    indexToElement.current.clear();
    scheduleRecompute();
  }, [count, scheduleRecompute]);

  const rowOffsets = useMemo(() => {
    const totalRows = count === 0 ? 0 : Math.ceil(count / columns);
    const offsets = new Float64Array(totalRows + 1);
    for (let row = 0; row < totalRows; row++) {
      let rowHeight: number;
      if (columns === 1) {
        rowHeight = heightSnapshot.get(row) ?? itemHeight;
      } else {
        const start = row * columns;
        const end = Math.min(count, start + columns);
        let measuredMax = 0;
        let anyMeasured = false;
        for (let i = start; i < end; i++) {
          const h = heightSnapshot.get(i);
          if (h !== undefined) {
            anyMeasured = true;
            if (h > measuredMax) measuredMax = h;
          }
        }
        rowHeight = anyMeasured ? measuredMax : itemHeight;
      }
      offsets[row + 1] = offsets[row] + rowHeight;
    }
    return offsets;
  }, [count, columns, itemHeight, heightSnapshot]);

  if (count === 0) {
    return { containerRef, virtualItems: [], paddingTop: 0, paddingBottom: 0, measureElement };
  }

  const totalRows = rowOffsets.length - 1;
  let startRow = findRowForOffset(rowOffsets, Math.max(0, scrollOffset));
  startRow = Math.max(0, startRow - overscan);

  let endRow = findRowForOffset(rowOffsets, Math.max(0, scrollOffset + viewportHeight));
  endRow = Math.min(totalRows - 1, endRow + overscan);

  const startIndex = startRow * columns;
  const endIndex = Math.min(count - 1, (endRow + 1) * columns - 1);

  const virtualItems: number[] = [];
  for (let i = startIndex; i <= endIndex; i++) {
    virtualItems.push(i);
  }

  const paddingTop = rowOffsets[startRow];
  const paddingBottom = rowOffsets[totalRows] - rowOffsets[endRow + 1];

  return {
    containerRef,
    virtualItems,
    paddingTop,
    paddingBottom,
    measureElement,
  };
}
