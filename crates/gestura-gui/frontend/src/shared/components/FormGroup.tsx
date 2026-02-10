import React from 'react';

export interface FormGroupProps extends React.HTMLAttributes<HTMLDivElement> {
  label?: React.ReactNode;
  hint?: React.ReactNode;
}

/**
 * Shared form group primitive.
 *
 * Mirrors the existing `.form-group` markup + styling in `App.css`.
 */
export function FormGroup({ label, hint, className, children, ...rest }: FormGroupProps) {
  const cn = ['form-group', className].filter(Boolean).join(' ');

  return (
    <div className={cn} {...rest}>
      {label ? <label>{label}</label> : null}
      {children}
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

