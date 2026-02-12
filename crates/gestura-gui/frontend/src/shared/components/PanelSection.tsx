import React from 'react';

export interface PanelSectionProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Optional heading rendered as an `h3` to match existing panel sections. */
  heading?: React.ReactNode;
}

/**
 * Shared panel section primitive.
 *
 * Wraps content in the existing `.panel` container and renders an optional `h3`.
 */
export function PanelSection({ heading, className, children, ...rest }: PanelSectionProps) {
  const cn = ['panel', className].filter(Boolean).join(' ');

  return (
    <div className={cn} {...rest}>
      {heading ? <h3>{heading}</h3> : null}
      {children}
    </div>
  );
}

