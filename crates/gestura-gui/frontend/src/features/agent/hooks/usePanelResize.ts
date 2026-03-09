/**
 * usePanelResize — drag-to-resize logic for a side panel.
 *
 * Returns a ref to attach to the drag handle and the current width so the
 * parent can apply it as an inline style on the panel element.
 *
 * @param initialWidth  Starting width in pixels.
 * @param minWidth      Minimum allowed width.
 * @param maxWidth      Maximum allowed width.
 * @param side          Which side the panel is on; determines drag direction.
 */
import { useCallback, useRef, useState } from 'react';

export type PanelSide = 'left' | 'right';

export interface UsePanelResizeReturn {
    width: number;
    setWidth: (w: number) => void;
    handleMouseDown: (e: React.MouseEvent) => void;
}

export function usePanelResize(
    initialWidth: number,
    minWidth: number,
    maxWidth: number,
    side: PanelSide,
): UsePanelResizeReturn {
    const [width, setWidth] = useState(initialWidth);
    const dragging = useRef(false);
    const startX = useRef(0);
    const startWidth = useRef(0);

    const handleMouseDown = useCallback(
        (e: React.MouseEvent) => {
            e.preventDefault();
            dragging.current = true;
            startX.current = e.clientX;
            startWidth.current = width;

            const onMove = (moveEvt: MouseEvent) => {
                if (!dragging.current) return;
                const delta = moveEvt.clientX - startX.current;
                // Left panels grow when dragging right; right panels grow when dragging left.
                const newWidth =
                    side === 'left'
                        ? startWidth.current + delta
                        : startWidth.current - delta;
                setWidth(Math.max(minWidth, Math.min(maxWidth, newWidth)));
            };

            const onUp = () => {
                dragging.current = false;
                window.removeEventListener('mousemove', onMove);
                window.removeEventListener('mouseup', onUp);
            };

            window.addEventListener('mousemove', onMove);
            window.addEventListener('mouseup', onUp);
        },
        [width, minWidth, maxWidth, side],
    );

    return { width, setWidth, handleMouseDown };
}
