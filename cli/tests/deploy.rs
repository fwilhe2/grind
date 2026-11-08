// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **The deployment samples name what the workflow actually publishes**, checked the way
//! `packaging.rs` checks that every binary is packaged: read the workflow and the samples, and
//! fail the build when they disagree.
//!
//! `ui_web/deploy/` holds one sample per thing people actually run — `compose.yaml` and
//! `kubernetes.yaml` — plus the two ways to reach that Service from outside a cluster,
//! `httproute.yaml` (Gateway API) and `ingress.yaml` (Ingress API), of which a given cluster
//! wants one. All of them are copy-and-run instructions rather than prose, which is exactly what
//! makes them rot silently: the image tag lives in `.github/workflows/container.yml`, the port
//! and the health endpoint are settings of the running server, and nothing about a stale sample
//! fails until somebody pastes it into a terminal. A sample nobody runs is `doc/plan.md` rule 4's
//! UI-only feature in another costume.
//!
//! What is checked here is only the agreement *between files* — the things one edit can break
//! in another file, plus one fact about the world that a comment would not hold anybody to:
//! `ingress-nginx` is retired, so neither routing sample may *apply* its annotations, however
//! much both of them discuss it.
//!
//! That the samples run at all is not a unit test's question: it needs a daemon and a cluster,
//! and it was answered by running them — Compose end to end, the pod spec under
//! `podman kube play`, and every manifest through `kubeconform -strict`, the Gateway API ones
//! against the CRD catalog's schemas.
//!
//! Everything is `include_str!`, read at compile time, so this cannot pass by looking in the
//! wrong place at runtime.

const WORKFLOW: &str = include_str!("../../.github/workflows/container.yml");
const COMPOSE: &str = include_str!("../../ui_web/deploy/compose.yaml");
const KUBERNETES: &str = include_str!("../../ui_web/deploy/kubernetes.yaml");
const HTTPROUTE: &str = include_str!("../../ui_web/deploy/httproute.yaml");
const INGRESS: &str = include_str!("../../ui_web/deploy/ingress.yaml");

/// A sample with the comments taken out — what the cluster is actually told, as opposed to what
/// the file explains. Both routing samples *discuss* `ingress-nginx` and its annotations at
/// length, so a check that nothing points at that controller has to be able to tell a line of
/// prose from a line of configuration.
fn live_lines(sample: &str) -> String {
    sample
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The image `container.yml`'s `web-image` job builds and pushes, read out of the workflow
/// rather than spelled a second time here.
fn published_image() -> &'static str {
    let line = WORKFLOW
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("IMAGE:") && l.contains("grind-web"))
        .expect("container.yml declares an IMAGE for the web shell");
    line.trim_start_matches("IMAGE:").trim()
}

#[test]
fn both_samples_deploy_the_image_the_workflow_publishes() {
    let image = published_image();
    assert_eq!(
        image, "ghcr.io/fwilhe2/grind-web",
        "the published web image moved; the samples in ui_web/deploy/ name the old one"
    );

    for (name, sample) in [("compose.yaml", COMPOSE), ("kubernetes.yaml", KUBERNETES)] {
        assert!(
            sample.contains(&format!("{image}:latest")),
            "ui_web/deploy/{name} does not deploy {image}:latest, which is what \
             container.yml publishes"
        );
    }
}

#[test]
fn the_source_build_is_the_dockerfile_the_workflow_builds() {
    // The compose file's `source` profile and the workflow's `web-image` job must build the
    // same thing the same way: this Dockerfile, with the repository root as the context,
    // because the page and grind are built from one tree.
    assert!(
        WORKFLOW.contains("file: ./ui_web/Dockerfile"),
        "container.yml no longer builds ui_web/Dockerfile"
    );
    assert!(
        COMPOSE.contains("dockerfile: ui_web/Dockerfile"),
        "compose.yaml's source profile builds a different Dockerfile than the workflow does"
    );
    assert!(
        COMPOSE.contains("context: ../.."),
        "compose.yaml's build context must be the repository root, not ui_web/"
    );
}

#[test]
fn grind_is_reached_on_loopback_and_only_there() {
    // `grind serve` listens on 127.0.0.1 alone (doc/local-only.md). A published port forwards to
    // an address it is not on, so Compose shares the host's network, and Kubernetes puts a proxy
    // in the pod: the Service's port is the proxy's, and grind's own container declares none.
    let compose = live_lines(COMPOSE);
    assert!(
        compose.contains("network_mode: host"),
        "compose.yaml must use the host's network"
    );
    assert!(
        !compose.contains("ports:"),
        "compose.yaml publishes a port grind does not listen on"
    );
    assert!(
        compose.contains("init: true"),
        "grind as PID 1 does not stop on SIGTERM"
    );

    let kubernetes = live_lines(KUBERNETES);
    assert!(
        kubernetes.contains(r#"args: ["serve", "/documents", "--port", "8080", "--host", "#),
        "kubernetes.yaml's grind no longer serves /documents on the port the proxy forwards to"
    );
    assert!(
        kubernetes.contains("reverse_proxy 127.0.0.1:8080"),
        "the proxy no longer forwards to grind's loopback port"
    );
    assert!(
        kubernetes.contains(":8081 {") && kubernetes.contains("containerPort: 8081"),
        "the proxy listens on a port its container does not declare"
    );
    assert_eq!(
        kubernetes.matches("containerPort:").count(),
        1,
        "only the proxy declares a port; grind has nothing to offer the pod's address"
    );
}

#[test]
fn the_probes_ask_grind_through_the_proxy() {
    // The kubelet probes the pod's address, which only the proxy is on, and with the pod's IP as
    // the Host, which grind refuses — so each probe names a host grind answers to.
    assert_eq!(
        KUBERNETES.matches("path: /api/v1/hello").count(),
        2,
        "kubernetes.yaml should probe the server from both the readiness and the liveness probe"
    );
    assert_eq!(
        KUBERNETES.matches("value: localhost").count(),
        2,
        "a probe without `Host: localhost` is refused by grind as another site's request"
    );
    // There is no shell in a distroless image, so an `exec` probe cannot work here.
    assert!(
        !KUBERNETES.contains("exec:"),
        "an exec probe cannot run in a distroless image; use httpGet"
    );
}

#[test]
fn the_documents_are_one_servers() {
    // Two servers over one directory would take each other's saves for somebody else's.
    let kubernetes = live_lines(KUBERNETES);
    for required in [
        "replicas: 1",
        "type: Recreate",
        "kind: PersistentVolumeClaim",
        "claimName: grind-documents",
        "mountPath: /documents",
        "fsGroup: 65532",
    ] {
        assert!(
            kubernetes.contains(required),
            "kubernetes.yaml no longer says `{required}`"
        );
    }
    assert!(
        !kubernetes.contains("PodDisruptionBudget"),
        "a budget of one available pod over one replica blocks every node drain"
    );
    assert!(
        COMPOSE.contains(":/documents:z"),
        "compose.yaml no longer mounts the documents"
    );
}

#[test]
fn the_pod_is_unprivileged_and_read_only() {
    // The namespace enforces the `restricted` Pod Security Standard, so a pod spec that loses
    // one of these stops being admitted rather than quietly running with more than it needs.
    for required in [
        "pod-security.kubernetes.io/enforce: restricted",
        "runAsNonRoot: true",
        "type: RuntimeDefault",
    ] {
        assert!(
            KUBERNETES.contains(required),
            "kubernetes.yaml no longer says `{required}`, which the restricted \
             Pod Security Standard requires of it"
        );
    }
    // Both containers, each of them.
    for required in [
        "readOnlyRootFilesystem: true",
        "allowPrivilegeEscalation: false",
        r#"drop: ["ALL"]"#,
    ] {
        assert_eq!(
            KUBERNETES.matches(required).count(),
            2,
            "kubernetes.yaml says `{required}` of fewer than both containers"
        );
    }

    for required in ["read_only: true", "- ALL", "no-new-privileges:true"] {
        assert!(
            COMPOSE.contains(required),
            "compose.yaml no longer says `{required}`"
        );
    }
}

#[test]
fn both_routing_samples_point_at_the_service_that_exists() {
    // Three files, one Service. The Ingress asks for it by the port's *name*, the HTTPRoute by
    // its number — that is the two APIs' own spelling, not a choice made here — so the Service
    // has to keep both.
    assert!(
        KUBERNETES.contains("kind: Service") && KUBERNETES.contains("targetPort: http"),
        "kubernetes.yaml's Service lost the named port the Ingress refers to"
    );
    assert!(
        KUBERNETES.contains("port: 80"),
        "kubernetes.yaml's Service lost port 80, which httproute.yaml's backendRef names"
    );

    for (name, sample) in [("httproute.yaml", HTTPROUTE), ("ingress.yaml", INGRESS)] {
        assert!(
            sample.contains("name: grind-web"),
            "ui_web/deploy/{name} does not route to the grind-web Service"
        );
        assert!(
            sample.contains("namespace: grind"),
            "ui_web/deploy/{name} disagrees with kubernetes.yaml about the namespace"
        );
    }

    assert!(
        HTTPROUTE.contains("backendRefs:") && HTTPROUTE.contains("port: 80"),
        "httproute.yaml's backendRef no longer names the Service's port"
    );
    assert!(
        INGRESS.contains("name: http"),
        "ingress.yaml no longer names the Service's http port"
    );

    // grind answers only to the names it was given, so the routes' host must be one of them.
    let host = r#""--host", "grind.example.com""#;
    assert!(
        KUBERNETES.contains(host),
        "kubernetes.yaml's --host is not the routes' host"
    );
    for (name, sample) in [("httproute.yaml", HTTPROUTE), ("ingress.yaml", INGRESS)] {
        assert!(
            live_lines(sample).contains("grind.example.com"),
            "ui_web/deploy/{name} serves a host kubernetes.yaml's --host does not name"
        );
    }
}

#[test]
fn the_gateway_api_sample_is_on_the_ga_group_version() {
    // GatewayClass, Gateway and HTTPRoute are GA. A sample that drifted back to an alpha or beta
    // group would be telling people to write something their cluster may no longer serve.
    assert!(
        HTTPROUTE.contains("apiVersion: gateway.networking.k8s.io/v1"),
        "httproute.yaml is not on the GA Gateway API group version"
    );
    for prerelease in [
        "gateway.networking.k8s.io/v1alpha2",
        "gateway.networking.k8s.io/v1beta1",
    ] {
        assert!(
            !live_lines(HTTPROUTE).contains(prerelease),
            "httproute.yaml uses the pre-GA group version {prerelease}"
        );
    }

    // The one redirect this file needs is HTTP to HTTPS, which is Core conformance — a scheme
    // redirect with a 301. Anything Extended (the subpath rewrite, a path redirect) stays
    // commented, since it needs the controller's conformance report checked first.
    assert!(
        HTTPROUTE.contains("type: RequestRedirect") && HTTPROUTE.contains("scheme: https"),
        "httproute.yaml lost the HTTP-to-HTTPS redirect"
    );
    assert!(
        !live_lines(HTTPROUTE).contains("URLRewrite"),
        "httproute.yaml promotes an Extended-conformance filter into the applied manifest; \
         the subpath variant is commented on purpose"
    );
}

#[test]
fn neither_routing_sample_points_at_the_retired_controller() {
    // `kubernetes/ingress-nginx` reached end of life on 31 March 2026: archived, no releases and
    // no security fixes, in something that sits in the HTTP data path. Both files *discuss* it —
    // that is the whole explanation for why there are two of them — so this reads what the
    // cluster is told rather than what the comments say.
    for (name, sample) in [("httproute.yaml", HTTPROUTE), ("ingress.yaml", INGRESS)] {
        let live = live_lines(sample);
        assert!(
            !live.contains("nginx.ingress.kubernetes.io/"),
            "ui_web/deploy/{name} applies an ingress-nginx annotation; that controller is retired"
        );
        assert!(
            !live.contains("ingressClassName: nginx"),
            "ui_web/deploy/{name} defaults to the retired ingress-nginx controller"
        );
    }
}
