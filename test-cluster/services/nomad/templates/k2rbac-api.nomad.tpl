job "[[job_name]]" {
  datacenters = ["*"]
  type        = "service"

  meta = {
    build_tag = "[[build_tag]]"
    roll_id   = "[[roll_id]]"
  }

  group "api" {
    count = 1

{% if excluded_host | default('') | string | trim | length > 0 %}
    constraint {
      attribute = "${attr.unique.hostname}"
      operator  = "!="
      value     = "[[excluded_host]]"
    }
{% endif %}

    update {
      canary            = 1
      auto_promote      = true
      auto_revert       = true
      max_parallel      = 1
      health_check      = "checks"
      min_healthy_time  = "15s"
      healthy_deadline  = "10m"
      progress_deadline = "15m"
    }

    network {
      port "http" {
        to = 4100
      }
    }

    service {
      name     = "k2rbac-api"
      provider = "consul"
      port     = "http"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:runtime"]

      check {
        name     = "ready"
        type     = "http"
        path     = "/ready"
        interval = "10s"
        timeout  = "2s"
      }
    }

    service {
      name     = "edge"
      provider = "consul"
      port     = "http"
      tags     = ["build:[[build_tag]]", "roll:[[roll_id]]", "plane:edge", "domain:[[auth_domain]]"]
      meta {
        domain = "[[auth_domain]]"
      }

      check {
        name     = "ready"
        type     = "http"
        path     = "/ready"
        interval = "10s"
        timeout  = "2s"
      }
    }

    task "server" {
      driver = "docker"

      config {
        image = "[[image_repo]]:[[image_tag]]"
        ports = ["http"]
        args = ["serve"]
      }

      template {
        destination = "${NOMAD_SECRETS_DIR}/app.env"
        env         = true
        change_mode = "restart"

        data = <<EOT
RBAC_API_LISTEN_HOST=0.0.0.0
RBAC_API_LISTEN_PORT=4100
      [[jwt_secret_template]]
{{- with service "k2db-api" }}
{{- with index . 0 }}
    RBAC_K2DB_BASE_URL=http://{{ .Address }}:{{ .Port }}
{{- end }}
{{- end }}
{% if mail_plane_template | default('') | trim | length > 0 %}
{{- with service "k2mx-api" }}
{{- with index . 0 }}
  RBAC_K2MX_BASE_URL=http://{{ .Address }}:{{ .Port }}
{{- end }}
{{- end }}
  RBAC_LOGIN_ORIGIN=https://[[login_domain]]
{% endif %}
      [[k2db_api_key_template]]
      [[mail_plane_template]]
EOT
      }

        [[vault_block]]

      resources {
        cpu    = 500
        memory = 250
      }

      restart {
        attempts = 3
        delay    = "10s"
        mode     = "fail"
      }
    }
  }
}