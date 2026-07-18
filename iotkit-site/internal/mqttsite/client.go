package mqttsite

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"log/slog"
	"net/url"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const (
	recordsTopicFilter          = "iotkit/v1/edge-nodes/+/records"
	descriptorsTopicFilter      = "iotkit/v1/edge-nodes/+/descriptors"
	activationResultTopicFilter = "iotkit/v1/edge-nodes/+/activation/result"
	convergenceInterval         = 250 * time.Millisecond
	convergenceBatchSize        = 256
	publishAcknowledgementTTL   = 15 * time.Second
	statusPublishInterval       = 30 * time.Second
)

type ClientConfig struct {
	BrokerURL     string
	ClientID      string
	Username      string
	Password      string
	TLSConfig     *tls.Config
	AllowInsecure bool
}

type pendingExportQueue interface {
	ListPendingMQTTExports(context.Context, int) ([]store.PendingMQTTExport, error)
	MarkMQTTExportPublished(context.Context, string) error
}

type ExportQueue interface {
	ListPendingMQTTExports(context.Context, int) ([]store.PendingMQTTExport, error)
	MarkMQTTExportPublished(context.Context, string) error
	ReconcileInventorySources(context.Context, int) (int, error)
	ProjectSemanticEvents(context.Context, int) (int, error)
	EnqueueMQTTExports(context.Context, int) (int, error)
}

type genericExportQueue interface {
	ProjectSemanticObservations(context.Context, int) (int, error)
	EnqueueOutputExports(context.Context, int) (int, error)
}

type activationCommandQueue interface {
	ListPendingActivationCommands(context.Context, int) ([]store.ActivationCommand, error)
	MarkActivationCommandAttempt(context.Context, string, int64) error
}

type yokakitStatusQueue interface {
	ListYokaKitSourceIDs(context.Context) ([]string, error)
}

type exportPublish func(topic string, qos byte, payload []byte) error
type activationPublish func(topic string, qos byte, retained bool, payload []byte) error

type publishToken interface {
	Done() <-chan struct{}
	Error() error
}

func Run(ctx context.Context, config ClientConfig, processor Processor, queue ExportQueue, logger *slog.Logger) error {
	return runSite(ctx, config, processor, queue, logger, true)
}

func RunIngest(ctx context.Context, config ClientConfig, processor Processor, queue ExportQueue, logger *slog.Logger) error {
	return runSite(ctx, config, processor, queue, logger, false)
}

func runSite(
	ctx context.Context,
	config ClientConfig,
	processor Processor,
	queue ExportQueue,
	logger *slog.Logger,
	publishOutputs bool,
) error {
	if err := config.validate(); err != nil {
		return err
	}
	if queue == nil {
		return errors.New("MQTT export queue is nil")
	}
	if logger == nil {
		logger = slog.Default()
	}

	handler := func(client mqtt.Client, message mqtt.Message) {
		err := processor.Process(context.Background(), message.Topic(), message.Payload(), func(topic string, payload []byte) error {
			return publishWithTimeout(context.Background(), func() publishToken {
				return client.Publish(topic, 1, false, payload)
			})
		})
		if err != nil {
			logger.Error("MQTT message processing failed", "topic", message.Topic(), "error", err)
		}
	}
	subscriptionResults := make(chan error, 1)

	options := newClientOptions(config)
	options.SetConnectionLostHandler(func(_ mqtt.Client, err error) {
		logger.Warn("MQTT connection lost", "error", err)
	})
	options.SetOnConnectHandler(func(client mqtt.Client) {
		token := client.SubscribeMultiple(map[string]byte{
			recordsTopicFilter:          1,
			descriptorsTopicFilter:      1,
			activationResultTopicFilter: 1,
		}, handler)
		var err error
		if !token.WaitTimeout(15 * time.Second) {
			err = errors.New("MQTT subscribe timed out")
		} else if tokenErr := token.Error(); tokenErr != nil {
			err = fmt.Errorf("MQTT subscribe: %w", tokenErr)
		}
		if err == nil {
			logger.Info("IoTKit Site subscribed", "topics", []string{
				recordsTopicFilter,
				descriptorsTopicFilter,
				activationResultTopicFilter,
			})
		}
		subscriptionResults <- err
	})

	client := mqtt.NewClient(options)
	if token := client.Connect(); !token.WaitTimeout(15 * time.Second) {
		return errors.New("MQTT connect timed out")
	} else if err := token.Error(); err != nil {
		return fmt.Errorf("MQTT connect: %w", err)
	}
	defer client.Disconnect(250)

	select {
	case err := <-subscriptionResults:
		if err != nil {
			return err
		}
	case <-time.After(15 * time.Second):
		return errors.New("MQTT initial subscription did not complete")
	case <-ctx.Done():
		return nil
	}

	ticker := time.NewTicker(convergenceInterval)
	defer ticker.Stop()
	statusTicker := time.NewTicker(statusPublishInterval)
	defer statusTicker.Stop()
	if publishOutputs {
		publishYokaKitStatuses(ctx, queue, func(topic string, qos byte, retained bool, payload []byte) error {
			return publishWithTimeout(ctx, func() publishToken {
				return client.Publish(topic, qos, retained, payload)
			})
		}, logger)
	}
	for {
		select {
		case err := <-subscriptionResults:
			if err != nil {
				return fmt.Errorf("MQTT resubscribe: %w", err)
			}
		case <-ticker.C:
			if activationQueue, ok := queue.(activationCommandQueue); ok {
				if err := publishPendingActivationCommands(
					ctx,
					activationQueue,
					func(topic string, qos byte, retained bool, payload []byte) error {
						return publishWithTimeout(ctx, func() publishToken {
							return client.Publish(topic, qos, retained, payload)
						})
					},
				); err != nil && !errors.Is(err, context.Canceled) {
					logger.Error("MQTT Edge activation command failed", "error", err)
				}
			}
			convergeSite(ctx, queue, logger)
			if publishOutputs {
				if err := publishPending(ctx, queue, func(topic string, qos byte, payload []byte) error {
					return publishWithTimeout(ctx, func() publishToken {
						return client.Publish(topic, qos, false, payload)
					})
				}); err != nil && !errors.Is(err, context.Canceled) {
					logger.Error("MQTT application export failed", "error", err)
				}
			}
		case <-statusTicker.C:
			if publishOutputs {
				publishYokaKitStatuses(ctx, queue, func(topic string, qos byte, retained bool, payload []byte) error {
					return publishWithTimeout(ctx, func() publishToken {
						return client.Publish(topic, qos, retained, payload)
					})
				}, logger)
			}
		case <-ctx.Done():
			return nil
		}
	}
}

func RunOutput(
	ctx context.Context,
	config ClientConfig,
	queue ExportQueue,
	logger *slog.Logger,
) error {
	if err := config.validate(); err != nil {
		return err
	}
	if queue == nil {
		return errors.New("MQTT output queue is nil")
	}
	if logger == nil {
		logger = slog.Default()
	}
	client := mqtt.NewClient(newClientOptions(config))
	if token := client.Connect(); !token.WaitTimeout(15 * time.Second) {
		return errors.New("MQTT output connect timed out")
	} else if err := token.Error(); err != nil {
		return fmt.Errorf("MQTT output connect: %w", err)
	}
	defer client.Disconnect(250)
	publish := func(topic string, qos byte, payload []byte) error {
		return publishWithTimeout(ctx, func() publishToken {
			return client.Publish(topic, qos, false, payload)
		})
	}
	statusPublish := func(topic string, qos byte, retained bool, payload []byte) error {
		return publishWithTimeout(ctx, func() publishToken {
			return client.Publish(topic, qos, retained, payload)
		})
	}
	ticker := time.NewTicker(convergenceInterval)
	defer ticker.Stop()
	statusTicker := time.NewTicker(statusPublishInterval)
	defer statusTicker.Stop()
	publishYokaKitStatuses(ctx, queue, statusPublish, logger)
	for {
		select {
		case <-ticker.C:
			if err := publishPending(ctx, queue, publish); err != nil &&
				!errors.Is(err, context.Canceled) {
				logger.Error("MQTT application export failed", "error", err)
			}
		case <-statusTicker.C:
			publishYokaKitStatuses(ctx, queue, statusPublish, logger)
		case <-ctx.Done():
			return nil
		}
	}
}

func publishYokaKitStatuses(
	ctx context.Context,
	queue ExportQueue,
	publish func(string, byte, bool, []byte) error,
	logger *slog.Logger,
) {
	statuses, ok := queue.(yokakitStatusQueue)
	if !ok {
		return
	}
	sourceIDs, err := statuses.ListYokaKitSourceIDs(ctx)
	if err != nil {
		logger.Error("YokaKit source status query failed", "error", err)
		return
	}
	for _, sourceID := range sourceIDs {
		message, err := outputadapter.YokaKitStatus(sourceID, time.Now().UnixMilli())
		if err == nil {
			err = publish(message.Topic, message.QoS, message.Retain, message.Payload)
		}
		if err != nil {
			logger.Error("YokaKit source status publish failed",
				"source_id", sourceID, "error", err)
		}
	}
}

func newClientOptions(config ClientConfig) *mqtt.ClientOptions {
	options := mqtt.NewClientOptions().
		AddBroker(config.BrokerURL).
		SetClientID(config.ClientID).
		SetUsername(config.Username).
		SetPassword(config.Password).
		SetCleanSession(true).
		SetAutoReconnect(true).
		SetConnectRetry(true).
		SetConnectRetryInterval(time.Second).
		SetOrderMatters(false).
		SetWriteTimeout(publishAcknowledgementTTL)
	if config.TLSConfig != nil {
		options.SetTLSConfig(config.TLSConfig)
	}
	return options
}

func publishWithTimeout(ctx context.Context, publish func() publishToken) error {
	deadline := time.Now().Add(publishAcknowledgementTTL)
	return publishWithDeadline(ctx, publish, deadline)
}

func publishWithDeadline(ctx context.Context, publish func() publishToken, deadline time.Time) error {
	remaining := time.Until(deadline)
	if remaining <= 0 {
		return errors.New("MQTT publish timed out")
	}
	timer := time.NewTimer(remaining)
	defer timer.Stop()
	tokens := make(chan publishToken, 1)
	go func() {
		tokens <- publish()
	}()

	select {
	case token := <-tokens:
		return waitForPublishCompletion(ctx, token, deadline)
	case <-timer.C:
		return errors.New("MQTT publish timed out")
	case <-ctx.Done():
		return ctx.Err()
	}
}

func waitForPublishCompletion(ctx context.Context, token publishToken, deadline time.Time) error {
	remaining := time.Until(deadline)
	if remaining <= 0 {
		return errors.New("MQTT publish timed out")
	}
	timer := time.NewTimer(remaining)
	defer timer.Stop()
	select {
	case <-token.Done():
		return token.Error()
	case <-timer.C:
		return errors.New("MQTT publish timed out")
	case <-ctx.Done():
		return ctx.Err()
	}
}

func convergeExports(ctx context.Context, queue ExportQueue, publish exportPublish, logger *slog.Logger) {
	convergeSite(ctx, queue, logger)
	if ctx.Err() != nil {
		return
	}
	if err := publishPending(ctx, queue, publish); err != nil && !errors.Is(err, context.Canceled) {
		logger.Error("MQTT application export failed", "error", err)
	}
}

func convergeSite(ctx context.Context, queue ExportQueue, logger *slog.Logger) {
	if ctx.Err() != nil {
		return
	}
	if _, err := queue.ReconcileInventorySources(ctx, convergenceBatchSize); err != nil && !errors.Is(err, context.Canceled) {
		logger.Error("inventory reconciliation failed", "error", err)
	}
	if ctx.Err() != nil {
		return
	}
	if _, err := queue.ProjectSemanticEvents(ctx, convergenceBatchSize); err != nil && !errors.Is(err, context.Canceled) {
		logger.Error("semantic projection failed", "error", err)
	}
	if generic, ok := queue.(genericExportQueue); ok {
		if _, err := generic.ProjectSemanticObservations(ctx, convergenceBatchSize); err != nil &&
			!errors.Is(err, context.Canceled) {
			logger.Error("generic semantic projection failed", "error", err)
		}
	}
	if ctx.Err() != nil {
		return
	}
	if _, err := queue.EnqueueMQTTExports(ctx, convergenceBatchSize); err != nil && !errors.Is(err, context.Canceled) {
		logger.Error("MQTT export enqueue failed", "error", err)
	}
	if generic, ok := queue.(genericExportQueue); ok {
		if _, err := generic.EnqueueOutputExports(ctx, convergenceBatchSize); err != nil &&
			!errors.Is(err, context.Canceled) {
			logger.Error("generic output enqueue failed", "error", err)
		}
	}
}

func publishPending(ctx context.Context, queue pendingExportQueue, publish exportPublish) error {
	pending, err := queue.ListPendingMQTTExports(ctx, convergenceBatchSize)
	if err != nil {
		return fmt.Errorf("list pending MQTT exports: %w", err)
	}
	failedRoutes := make(map[string]struct{})
	var publishErrors []error
	for _, item := range pending {
		if err := ctx.Err(); err != nil {
			return err
		}
		if _, failed := failedRoutes[item.RouteID]; failed {
			continue
		}
		if item.QoS != 1 {
			failedRoutes[item.RouteID] = struct{}{}
			publishErrors = append(publishErrors,
				fmt.Errorf("MQTT export %q has unsupported QoS %d", item.ExportID, item.QoS))
			continue
		}
		if err := publish(item.Topic, 1, item.PayloadJSON); err != nil {
			if ctxErr := ctx.Err(); ctxErr != nil {
				return ctxErr
			}
			failedRoutes[item.RouteID] = struct{}{}
			publishErrors = append(publishErrors,
				fmt.Errorf("publish MQTT export %q: %w", item.ExportID, err))
			continue
		}
		if err := queue.MarkMQTTExportPublished(ctx, item.ExportID); err != nil {
			if ctxErr := ctx.Err(); ctxErr != nil {
				return ctxErr
			}
			failedRoutes[item.RouteID] = struct{}{}
			publishErrors = append(publishErrors,
				fmt.Errorf("mark MQTT export %q published: %w", item.ExportID, err))
		}
	}
	return errors.Join(publishErrors...)
}

func publishPendingActivationCommands(
	ctx context.Context,
	queue activationCommandQueue,
	publish activationPublish,
) error {
	pending, err := queue.ListPendingActivationCommands(ctx, convergenceBatchSize)
	if err != nil {
		return fmt.Errorf("list pending Edge activation commands: %w", err)
	}
	var publishErrors []error
	for _, command := range pending {
		if err := ctx.Err(); err != nil {
			return err
		}
		if err := publish(command.Topic, 1, false, command.PayloadJSON); err != nil {
			if ctxErr := ctx.Err(); ctxErr != nil {
				return ctxErr
			}
			publishErrors = append(
				publishErrors,
				fmt.Errorf("publish Edge activation command %q: %w", command.ActivationID, err),
			)
			continue
		}
		if err := queue.MarkActivationCommandAttempt(
			ctx,
			command.ActivationID,
			time.Now().UnixMilli(),
		); err != nil {
			if ctxErr := ctx.Err(); ctxErr != nil {
				return ctxErr
			}
			publishErrors = append(
				publishErrors,
				fmt.Errorf("record Edge activation attempt %q: %w", command.ActivationID, err),
			)
		}
	}
	return errors.Join(publishErrors...)
}

func (config ClientConfig) validate() error {
	if config.BrokerURL == "" || config.ClientID == "" {
		return errors.New("MQTT broker URL and client ID are required")
	}
	if config.Username == "" || config.Password == "" {
		return errors.New("MQTT username and password file content are required")
	}
	parsed, err := url.Parse(config.BrokerURL)
	if err != nil {
		return fmt.Errorf("parse MQTT broker URL: %w", err)
	}
	switch parsed.Scheme {
	case "ssl", "tls", "mqtts":
		if config.TLSConfig == nil {
			return errors.New("TLS broker requires TLS configuration")
		}
	case "tcp":
		if !config.AllowInsecure {
			return errors.New("plain MQTT requires explicit allow-insecure for local testing")
		}
	default:
		return fmt.Errorf("unsupported MQTT broker URL scheme %q", parsed.Scheme)
	}
	return nil
}
