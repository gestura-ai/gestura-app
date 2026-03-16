import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import HelloWorldPanel from './HelloWorldPanel';

describe('HelloWorldPanel', () => {
  it('renders the hello world message', () => {
    render(<HelloWorldPanel />);

    expect(screen.getByRole('heading', { name: 'Hello, World!' })).toBeInTheDocument();
    expect(
      screen.getByText('This is a small Gestura Tauri GUI screen rendered with React.'),
    ).toBeInTheDocument();
  });
});