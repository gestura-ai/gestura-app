import React from 'react';

export type ButtonTone = 'default' | 'secondary' | 'primary' | 'danger' | 'gradient';
export type ButtonSize = 'md' | 'small' | 'large' | 'icon';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  tone?: ButtonTone;
  size?: ButtonSize;
}

function toClassName(tone: ButtonTone | undefined, size: ButtonSize | undefined, className: string | undefined): string {
  const classes: Array<string | undefined> = ['btn'];

  switch (tone) {
    case 'secondary':
      classes.push('btn-secondary');
      break;
    case 'primary':
      classes.push('btn-primary');
      break;
    case 'danger':
      classes.push('btn-danger');
      break;
    case 'gradient':
      classes.push('btn-gradient');
      break;
    case 'default':
    default:
      break;
  }

  switch (size) {
    case 'small':
      classes.push('btn-small');
      break;
    case 'large':
      classes.push('btn-large');
      break;
    case 'icon':
      classes.push('btn-icon');
      break;
    case 'md':
    default:
      break;
  }

  if (className) classes.push(className);

  return classes.filter(Boolean).join(' ');
}

/**
 * Shared button primitive.
 *
 * Uses the existing global `.btn*` classes in `App.css` to avoid style regressions.
 */
export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { tone = 'default', size = 'md', className, type = 'button', ...rest },
  ref
) {
  return <button ref={ref} type={type} className={toClassName(tone, size, className)} {...rest} />;
});

