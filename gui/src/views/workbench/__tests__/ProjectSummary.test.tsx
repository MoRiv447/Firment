import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ProjectSummary } from '../ProjectSummary';
import type { WorkbenchStateDto } from '../../../types';

/**
 * The card that reads `.firment/workbench.toml` back to the user.
 *
 * What is being proven here is the newest part: a file that exists and does not parse used to be
 * indistinguishable from a project with nothing in it, because the backend answered with the
 * default struct and no complaint. `config_error` is the complaint, and the panel is where it
 * either reaches the person or does not.
 */
function state(over: Partial<WorkbenchStateDto> = {}, configOver = {}): WorkbenchStateDto {
  return {
    config: {
      project_name: 'fw-thermostat',
      mainline_session: 'e5bd87a2-057c-4a3e-a87b-7cb5bf6b3335',
      guard_escalate_sev: 'warn',
      toml_raw: '[project]\nname = "fw-thermostat"\n',
      config_error: null,
      ...configOver,
    },
    git: { branch: 'main', dirty_files: 2 },
    root: '/home/dev/fw-thermostat',
    ...over,
  };
}

describe('ProjectSummary', () => {
  it('names the project, the root and the mainline of a readable file', () => {
    render(<ProjectSummary state={state()} />);
    expect(screen.getByText('Project: fw-thermostat')).toBeInTheDocument();
    expect(screen.getByText('/home/dev/fw-thermostat')).toBeInTheDocument();
    expect(screen.getByText('e5bd87a2')).toBeInTheDocument();
    // The negative control for the rest of this file: a healthy config shows no complaint.
    expect(screen.queryByText(/does not parse/)).toBeNull();
  });

  it('says out loud when the file exists but cannot be parsed', () => {
    // The parse failure is the whole message: without it the card shows an unnamed project with no
    // pins and no mainline, and the only true thing -- "your file is broken" -- is nowhere.
    render(
      <ProjectSummary
        state={
          state({}, {
            project_name: '',
            mainline_session: '',
            toml_raw: '[pinmap]\nPA5 = { func = \n',
            config_error: 'corrupt /home/dev/fw/.firment/workbench.toml: expected value',
          })
        }
      />,
    );
    expect(screen.getByText('.firment/workbench.toml does not parse')).toBeInTheDocument();
    expect(screen.getByText(/expected value/)).toBeInTheDocument();
    // It has to say that the empty-looking values are not the file's, or the user "fixes" a project
    // name that was never missing.
    expect(screen.getByText(/values below are defaults/)).toBeInTheDocument();
    expect(screen.getByText('Project: (unnamed)')).toBeInTheDocument();
    // ...and still shows the bytes, which is what the person needs to repair the file.
    expect(screen.getByText(/\[pinmap\]/)).toBeInTheDocument();
  });

  it('does not claim a default-filled card is a corrupt one when the file is simply absent', () => {
    // A fresh project is a normal state, and warning about it would teach the user to ignore the
    // warning that matters.
    render(
      <ProjectSummary
        state={
          state({}, { project_name: '', mainline_session: '', toml_raw: '', config_error: null })
        }
      />,
    );
    expect(screen.queryByText(/does not parse/)).toBeNull();
    expect(screen.getByText(/No .firment\/workbench.toml yet/)).toBeInTheDocument();
  });
});
