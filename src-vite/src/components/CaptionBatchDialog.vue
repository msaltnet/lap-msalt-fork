<template>
  <ModalDialog :title="$t('msgbox.caption_batch.title')" :width="440" @cancel="handleDismiss">
    <div class="flex flex-col gap-4 select-none">
      <div class="space-y-2">
        <div class="flex items-center justify-between text-xs">
          <span class="font-medium text-base-content/70">
            {{ $t('msgbox.caption_batch.progress', {
              current: progress.current.toLocaleString(),
              total: progress.total.toLocaleString(),
            }) }}
          </span>
          <span class="tabular-nums text-base-content/40">{{ progressPercent }}%</span>
        </div>
        <progress
          class="progress progress-primary h-2 w-full"
          :value="progress.current"
          :max="Math.max(1, progress.total)"
        ></progress>
      </div>

      <div class="grid grid-cols-3 gap-2">
        <div class="rounded-box border border-base-content/5 bg-base-100/30 px-3 py-2">
          <div class="text-[10px] uppercase tracking-wide text-base-content/35">
            {{ $t('msgbox.caption_batch.succeeded') }}
          </div>
          <div class="mt-1 text-lg font-semibold tabular-nums text-success">
            {{ progress.succeeded.toLocaleString() }}
          </div>
        </div>
        <div class="rounded-box border border-base-content/5 bg-base-100/30 px-3 py-2">
          <div class="text-[10px] uppercase tracking-wide text-base-content/35">
            {{ $t('msgbox.caption_batch.skipped') }}
          </div>
          <div class="mt-1 text-lg font-semibold tabular-nums text-base-content/60">
            {{ progress.skipped.toLocaleString() }}
          </div>
        </div>
        <div class="rounded-box border border-base-content/5 bg-base-100/30 px-3 py-2">
          <div class="text-[10px] uppercase tracking-wide text-base-content/35">
            {{ $t('msgbox.caption_batch.failed') }}
          </div>
          <div class="mt-1 text-lg font-semibold tabular-nums" :class="progress.failed ? 'text-error' : 'text-base-content/60'">
            {{ progress.failed.toLocaleString() }}
          </div>
        </div>
      </div>

      <p v-if="cancelling" class="text-xs leading-5 text-warning">
        {{ $t('msgbox.caption_batch.cancelling') }}
      </p>

      <div class="flex justify-end">
        <button
          v-if="!isFinished"
          class="t-button-default"
          :disabled="cancelling"
          @click="emit('cancel')"
        >
          {{ cancelling ? $t('msgbox.caption_batch.cancelling') : $t('msgbox.caption_batch.cancel') }}
        </button>
        <button v-else class="t-button-primary" @click="emit('close')">
          {{ $t('msgbox.caption_batch.close') }}
        </button>
      </div>
    </div>
  </ModalDialog>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import ModalDialog from '@/components/ModalDialog.vue';

type CaptionBatchProgress = {
  total: number;
  current: number;
  succeeded: number;
  skipped: number;
  failed: number;
  cancelled: boolean;
};

const props = defineProps({
  progress: {
    type: Object as () => CaptionBatchProgress,
    required: true,
  },
  cancelling: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(['cancel', 'close']);
const isFinished = computed(() => (
  props.progress.cancelled
  || (props.progress.total > 0 && props.progress.current >= props.progress.total)
));
const progressPercent = computed(() => (
  props.progress.total > 0
    ? Math.round((props.progress.current / props.progress.total) * 100)
    : 0
));

function handleDismiss() {
  if (isFinished.value) emit('close');
  else if (!props.cancelling) emit('cancel');
}
</script>
