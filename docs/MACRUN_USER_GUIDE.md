# macrun User Guide

macrun remembers local secrets in macOS Keychain and gives them to a command when asked.

## Save a secret

For the whole machine:

```bash
macrun set API_TOKEN
```

For a project:

```bash
macrun set myapp API_TOKEN
```

For a project environment:

```bash
macrun set myapp staging API_TOKEN
```

macrun securely prompts for the value. A later `set` with the same scope and name replaces it.

## Run a command

```bash
macrun run -- some-command
macrun run myapp -- some-command
macrun run myapp staging -- some-command
```

The selected secrets are added to that command's environment. macrun stays quiet and returns the command's exit status.

## List names

```bash
macrun list
macrun list myapp
macrun list myapp staging
```

`list` prints names, never values.

## Unset a secret

```bash
macrun unset API_TOKEN
macrun unset myapp API_TOKEN
macrun unset myapp staging API_TOKEN
```

Unsetting a name that is already absent is harmless.

## Use macrun from a script

Send the value through standard input:

```bash
printf '%s' "$API_TOKEN" | macrun set myapp staging API_TOKEN --stdin
```

Or name an environment variable containing it:

```bash
macrun set myapp staging API_TOKEN --from-env API_TOKEN
```

Both avoid putting the value in macrun's command arguments.

## Ansible

Ansible can send a value directly to macrun on the controller Mac:

```yaml
- name: Store the local API token
  ansible.builtin.command:
    argv:
      - macrun
      - set
      - myapp
      - staging
      - API_TOKEN
      - --stdin
    stdin: "{{ api_token }}"
    stdin_add_newline: false
  delegate_to: localhost
  no_log: true
```

The secret still needs a safe source, such as Ansible Vault or an external credential system. `no_log: true` keeps the task value out of normal Ansible output.

## Terraform

Terraform 1.10 and later supports ephemeral variables that are omitted from plan and state. Such a value can be passed to macrun through a provisioner's environment:

```hcl
variable "api_token" {
  type      = string
  sensitive = true
  ephemeral = true
}

resource "terraform_data" "local_api_token" {
  provisioner "local-exec" {
    command = "macrun set myapp staging API_TOKEN --from-env MACRUN_VALUE"
    quiet   = true

    environment = {
      MACRUN_VALUE = var.api_token
    }
  }
}
```

macrun only protects its own destination. Do not use an ordinary Terraform value and assume macrun will keep it out of Terraform state.

## Scope summary

| Words before the secret or command | Scope |
| --- | --- |
| none | this machine |
| `PROJECT` | that project |
| `PROJECT ENVIRONMENT` | that project environment |

No initialization or project file is required.
